import json

import pytest

from hil import corpus



class _FakeClient:
    def __init__(self, base="http://dut.invalid"):
        self.base = base
        self.adapter = 0
        self.frames = []

    def raw(self, frame, expects_backward=False):
        self.frames.append(frame)
        return {"success": False, "backward_frame": 0}


def test_a_sweep_never_reaches_the_owners_live_luminaires(tmp_path):
    client = _FakeClient()
    cap = corpus.Capture(client, tmp_path, "primary")

    cap.gear([0, 4, 7, 12])

    assert client.frames, "the allowed fixture should still have been swept"
    for frame in client.frames:
        addr = (frame >> 8) & 0xFF
        if addr in (0xA3, 0xC1):
            continue
        assert (addr >> 1) in corpus.BENCH_FIXTURE_SHORTS, (
            "frame 0x%04X addresses short %d" % (frame, addr >> 1))
    refusals = [f for f in cap.failures if f["artifact"] == "gear:skipped"]
    assert len(refusals) == 1, "a refusal must be recorded, not silent"
    assert "4,7,12" in refusals[0]["body"]


def test_an_unanswered_query_records_null_and_never_zero(tmp_path):
    cap = corpus.Capture(_FakeClient(), tmp_path, "primary")

    entry = cap._exchange(0, 0x90, None)

    assert entry["answered"] is False
    assert entry["byte"] is None
    assert entry["name"] == "QUERY STATUS"



def test_wide_and_defined_colour_values_match_the_product():
    assert corpus._colour_value_is_wide(0)
    assert corpus._colour_value_is_wide(8)
    assert not corpus._colour_value_is_wide(9)
    assert corpus._colour_value_is_wide(128)
    assert not corpus._colour_value_is_wide(208)
    assert corpus._colour_value_is_wide(232)
    assert not corpus._colour_value_is_wide(233)

    defined = corpus._defined_colour_values()
    assert defined == sorted(set(defined)), "no duplicates"
    assert 15 in defined and 82 in defined and 208 in defined and 240 in defined
    assert 16 not in defined and 63 not in defined and 241 not in defined
    assert len(defined) == 73



def test_two_different_names_cannot_collapse_onto_one_file():
    assert corpus._slug("a0/physical_devices") == "a0%2Fphysical_devices"
    assert corpus._slug("Балкон.Стол") != corpus._slug("Балкон.Стол2")
    assert corpus._slug("plain-name_1.json") == "plain-name_1.json"



def test_a_board_that_could_not_be_asked_is_not_reported_as_rebooted():
    assert corpus._rebooted(100, 40) is True
    assert corpus._rebooted(100, 140) is False
    assert corpus._rebooted(None, 140) is False
    assert corpus._rebooted(100, None) is False



SAMPLE_LOG = """\
2026-09-16T00:00:01.000 I (1) x: DALI sniff: bad capture=[0f 0f 1e 1e]
2026-09-16T00:00:02.000 W (2) x: DALI sniff timing: isr ticks lost=0 extra=0
2026-09-16T00:00:03.000 D (3) x: DALI arbitration: answered 0xfffe3d (351 §7)
2026-09-16T00:00:04.000 I (4) x: task stack hwm (B free, 35 tasks): dali-sniff=5564
2026-09-16T00:00:05.000 W (5) x: boot heap [composition_start]: internal free=396000 largest=1
2026-09-16T00:00:06.000 I (6) x: nothing interesting here at all
2026-09-16T00:00:07.000 W (7) x: DALI line held: 31 run
2026-09-16T00:00:08.000 I (8) x: DALI sniff: bad capture=[aa bb]
"""


def _write_log(tmp_path, text):
    log = tmp_path / "serial.log"
    log.write_text(text)
    return log


def test_every_class_is_found_and_uninteresting_lines_are_dropped(tmp_path):
    out = tmp_path / "corpus"
    report = corpus.capture_wire(_write_log(tmp_path, SAMPLE_LOG), out)

    classes = report["classes"]
    assert set(classes) == {"captures", "sniff_timing", "arbitration",
                            "stack_census", "boot_heap", "line_held"}
    assert classes["captures"]["seen_in_window"] == 2
    assert report["truncated"] is False
    assert (out / "wire" / "captures.log").read_text().count("capture=[") == 2


def test_a_missing_log_is_reported_rather_than_invented(tmp_path):
    report = corpus.capture_wire(tmp_path / "absent.log", tmp_path / "corpus")
    assert report == {"log": str(tmp_path / "absent.log"), "present": False}


def test_the_cap_keeps_the_newest_lines_and_still_counts_the_rest(tmp_path):
    many = "".join("t%d x: DALI line held: %d run\n" % (i, i) for i in range(50))
    report = corpus.capture_wire(_write_log(tmp_path, many), tmp_path / "c",
                                 keep=10)

    held = report["classes"]["line_held"]
    assert (held["lines"], held["seen_in_window"]) == (10, 50)
    kept = (tmp_path / "c" / "wire" / "line_held.log").read_text()
    assert "49 run" in kept and "39 run" not in kept


def test_a_partial_scan_says_it_is_partial(tmp_path):
    report = corpus.capture_wire(_write_log(tmp_path, SAMPLE_LOG),
                                 tmp_path / "c", scan_bytes=120)
    assert report["truncated"] is True
    assert report["scanned_bytes"] < report["file_bytes"]


def test_per_class_caps_scale_with_the_requested_keep():
    assert corpus._keep_for("line_held", 2000) == 2000
    assert corpus._keep_for("captures", 2000) == corpus.WIRE_KEEP_BY_CLASS["captures"]
    assert corpus._keep_for("captures", 1000) == corpus.WIRE_KEEP_BY_CLASS["captures"] // 2
    assert corpus._keep_for("stack_census", 1) >= 1



def test_the_credential_bearing_slice_is_withheld_but_recorded(tmp_path):
    class _SliceClient(_FakeClient):
        manifest = [{"name": "poller_settings", "bytes": 9, "crc32": 1},
                    {"name": "home_assistant_settings", "bytes": 56, "crc32": 2},
                    {"name": "rules_b2", "bytes": None, "crc32": 0}]

        def raw_response(self, method, path, body=None):
            class _R:
                status_code = 200
                content = (json.dumps(_SliceClient.manifest).encode()
                           if path == "config/slices" else b"\x01\x02\x03")
            return _R()

    cap = corpus.Capture(_SliceClient(), tmp_path, "primary")
    cap.slices()

    assert "slice:poller_settings" in cap.artifacts
    assert "slice:home_assistant_settings" not in cap.artifacts
    assert "slice:rules_b2" not in cap.artifacts
    withheld = [f for f in cap.failures if f["status"] == "withheld"]
    assert len(withheld) == 1
    assert "--keep-secrets" in withheld[0]["body"]


def test_keep_secrets_includes_it(tmp_path):
    class _Client(_FakeClient):
        def raw_response(self, method, path, body=None):
            class _R:
                status_code = 200
                content = (b'[{"name":"home_assistant_settings","bytes":4,"crc32":0}]'
                           if path == "config/slices" else b"\xde\xad\xbe\xef")
            return _R()

    cap = corpus.Capture(_Client(), tmp_path, "primary")
    cap.slices(keep_secrets=True)

    assert cap.artifacts["slice:home_assistant_settings"]["bytes"] == 4
    assert not cap.failures



def test_a_board_that_does_not_answer_is_recorded_not_raised(tmp_path):
    import requests

    class _DeadClient(_FakeClient):
        def raw_response(self, method, path, body=None):
            raise requests.exceptions.ConnectTimeout("connect timeout")

    cap = corpus.Capture(_DeadClient(), tmp_path, "primary")

    identity = cap.identity()

    assert identity["version"] is None
    assert identity["base"] == "http://dut.invalid"
    failures = [f for f in cap.failures if f["artifact"].startswith("probe:")]
    assert failures, "the unreachable probe must land in the failure list"
    assert "ConnectTimeout" in failures[0]["body"]


def test_bench_identity_records_a_file_that_is_not_there(tmp_path):
    from hil.config import HilConfig
    import dataclasses

    state = tmp_path / "state"
    (state / "masks").mkdir(parents=True)
    (state / "camera_bench.json").write_text("{}")
    (state / "masks" / "roi0.png").write_bytes(b"\x89PNG")
    cfg = dataclasses.replace(HilConfig(), state_dir=state)
    cap = corpus.Capture(_FakeClient(), tmp_path / "out", "primary")

    corpus.capture_bench_identity(cfg, cap)

    assert "bench:camera_bench.json" in cap.artifacts
    assert "bench:masks/roi0.png" in cap.artifacts
    absent = [f for f in cap.failures if f["status"] == "absent"]
    assert any("calibration.json" in f["artifact"] for f in absent)


def test_late_tick_lines_are_classified_under_both_spellings():
    old = "W (1) dali: DALI ISR late tick: gap 3582 us (~34 alarms coalesced)"
    new = ("W (2) dali: DALI ISR late entries (delayed; see raw deficit for "
           "losses): ws_worker×1 (max 224 us @0x4ff0efe2<0x4ff0c272)")
    rx = dict(corpus.WIRE_CLASSES)["isr_late"]
    assert rx.search(old) and rx.search(new)


def test_boot_identity_lines_are_classified():
    classes = dict(corpus.WIRE_CLASSES)
    assert classes["flash_id"].search("I (80) dali2rust: flash: jedec=0xc84019")
    assert classes["flash_id"].search("I (2086) spi_flash: detected chip: gd")
    assert classes["phy_interrupt"].search(
        "I (900) esp_idf: DALI PHY interrupt: level 5, cpu int 17, core 0")
