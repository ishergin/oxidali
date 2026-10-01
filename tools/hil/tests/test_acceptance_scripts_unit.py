import ast
import datetime
import importlib
import importlib.util
from pathlib import Path

import pytest

from hil import provoke, seriallog, serialmon
from hil.config import HilConfig

HIL_ROOT = Path(__file__).resolve().parent.parent
TOOLKIT_SCRIPTS = sorted(HIL_ROOT.glob("*.py"))
BENCH_MARKS = ("192.168.", "/Users/")
BOOT = "I (1) dali2rust: boot\n"
HCL_STALE = "W (2) redundancy: worker has not turned for 61 s: hcl-scheduler"
QUALIFYING = [{"d_ticks": 1, "d_timeouts": 2}]
TIMING = ("2026-09-26T08:00:00.000Z I (3) x: DALI sniff timing: stage max=7 ticks "
          "over budget=2 of 40 poll gap max=900 us")
FLUSH = "2026-09-26T08:00:01.000Z W (4) x: slow flush_dirty_slices took 310 ms"


def _script(name):
    spec = importlib.util.spec_from_file_location(name, HIL_ROOT / ("%s.py" % name))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ISSUE86 = _script("issue86_acceptance")
FLUSH_STALL = _script("flush_stall_ab")
SOAK = _script("soak_abab")
LATE = ("2026-09-26T08:00:02.000Z W (5) dali: DALI ISR late entries (delayed; see raw deficit "
        "for losses): ws-client×2 (max 140 us @0x40001234<0x40005678), (none)×1 (max 9 us "
        "@0x0<0x0), httpd+isr×1 (max 60 us @0x1<0x2), ws-client+isr×1 (max 300 us @0x3<0x4)")
BENCH_UNREGISTERED = (
    "2026-09-17T15:38:24.560 W (14196) dali2rust_adapters::dali::transport::esp_idf: DALI ISR "
    "late entries (delayed; see raw deficit for losses): mqtt_worker×1 (max 220 us "
    "@0x4ff0efe2<0x4ff12416), dali-sniff×1 (max 251 us @0x4ff01f68<0x4ff0f0c4), 0x4ff89254 "
    "(not registered)×1 (max 217 us @0x4ff0efe2<0x4ff0c272)")
BENCH_INTERLEAVED = (
    "2026-09-22T21:43:14.820 W (3950143) dali2rust_adapters::dali::transport::esp_idf: DALI "
    "ISR late entries (delayed; see raw deficit for losses): ota-run×3 (max 330 us "
    "@0x4ff0f11c<0x4ff0dd16), ipc0×3 (max 306E (3952207) task_wdt: Task watchdog got "
    "triggered. The following tasks/users did not reset the watchdog in time:")


def _cfg(tmp_path, lamp_shorts="0,2,3"):
    return HilConfig(state_dir=tmp_path, serial_remote="", base="http://127.0.0.1:9",
                     lamp_shorts=lamp_shorts)


def test_issue86_reads_the_log_its_own_monitor_writes(tmp_path):
    log = tmp_path / "elsewhere" / "serial.log"
    log.parent.mkdir()
    log.write_text(BOOT)
    (tmp_path / "serial_monitor.pid.log").write_text(str(log))
    assert ISSUE86.serial_start(_cfg(tmp_path)) == (log, len(BOOT))


def test_issue86_passes_nothing_on_a_window_the_log_never_grew_into(tmp_path):
    log = tmp_path / "serial.log"
    assert ISSUE86.serial_window(log, 0) is None
    log.write_text(BOOT)
    start = log.stat().st_size
    assert ISSUE86.serial_window(log, start) is None
    assert not ISSUE86.verdict(QUALIFYING, False, None, 0)


def test_issue86_judges_a_window_the_log_did_grow_into(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(BOOT)
    start = log.stat().st_size
    with log.open("a") as fh:
        fh.write(BOOT)
    assert ISSUE86.serial_window(log, start) == []
    assert ISSUE86.verdict(QUALIFYING, False, [], 0)
    with log.open("a") as fh:
        fh.write(HCL_STALE + "\n")
    stale = ISSUE86.serial_window(log, start)
    assert stale == [HCL_STALE]
    assert not ISSUE86.verdict(QUALIFYING, False, stale, 0)


def _hcl_sample(t, ticks, worker_stale):
    return {"t": t, "ticks": ticks, "command_timeouts": 0, "commands_published": 0,
            "command_failures": 0, "worker_stale": worker_stale}


def test_issue86_never_ends_a_window_on_a_sample_with_a_missing_read():
    samples = [_hcl_sample(0.0, 1, 0), {"t": 3.0, "error": "diagnostics"},
               _hcl_sample(5.0, 2, None), _hcl_sample(10.0, 2, 0), _hcl_sample(15.0, 3, 0)]
    windows = ISSUE86._tick_windows(samples)
    assert [(w["ticks_from"], w["ticks_to"]) for w in windows] == [(1, 2), (2, 3)]
    assert [w["gaps"] for w in windows] == [2, 0]


def test_the_flush_stall_ab_reads_its_windows_by_offset_whatever_the_stamps(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(BOOT)
    start = FLUSH_STALL.log_size(log)
    assert FLUSH_STALL.log_lines(log, start) is None
    with log.open("a") as fh:
        fh.write("%s\n%s\nnoise\n" % (TIMING, FLUSH))
    assert FLUSH_STALL.window(FLUSH_STALL.log_lines(log, start)) == ([7], 2, 40, [900], [310])


class _Registry:
    def __init__(self, record):
        self.record, self.patches = dict(record), []

    def state(self, short):
        return dict(self.record)

    def device_patch(self, short, body):
        self.patches.append(body)
        self.record.update(body)

    def devices(self):
        return {"physical_devices": [{"short_address": s} for s in (1, 2, 4)]}


def test_the_flush_stall_ab_provokes_with_notes_it_restores_and_never_writes_a_name():
    registry = _Registry({"name": "Kitchen", "notes": "the owner's note"})
    toggle = FLUSH_STALL.NotesToggle(registry, 2)
    for _ in range(3):
        toggle()
    assert registry.record["notes"] == FLUSH_STALL.MARKER
    toggle.restore()
    assert registry.record == {"name": "Kitchen", "notes": "the owner's note"}
    assert all(set(body) == {"notes"} for body in registry.patches)


def test_the_flush_stall_ab_writes_only_gear_the_bench_may_write(tmp_path):
    assert FLUSH_STALL.probe_target(_Registry({}), _cfg(tmp_path)) == 2
    with pytest.raises(SystemExit, match="HIL_LAMP_SHORTS"):
        FLUSH_STALL.probe_target(_Registry({}), _cfg(tmp_path, lamp_shorts="7"))


def test_no_toolkit_script_names_a_bench_address_or_a_checkout_path():
    named = [path.name for path in TOOLKIT_SCRIPTS
             if any(mark in path.read_text() for mark in BENCH_MARKS)]
    assert named == []


def _missing_from(module, names):
    imported = importlib.import_module(module)
    return [name for name in names if not hasattr(imported, name) and not (
        hasattr(imported, "__path__")
        and importlib.util.find_spec("%s.%s" % (module, name)) is not None)]


def test_every_name_a_toolkit_script_imports_from_hil_exists():
    missing = []
    for path in TOOLKIT_SCRIPTS:
        for node in ast.walk(ast.parse(path.read_text())):
            if isinstance(node, ast.ImportFrom) and (node.module or "").split(".")[0] == "hil":
                missing += ["%s: %s.%s" % (path.name, node.module, name) for name in
                            _missing_from(node.module, [alias.name for alias in node.names])]
    assert missing == []


def test_late_entries_are_tallied_by_task_and_nesting():
    found = seriallog.late_entries([LATE, TIMING, LATE])
    assert found.tasks == {"ws-client": [4, 140], "(none)": [2, 9], "httpd+isr": [2, 60],
                           "ws-client+isr": [2, 300]}
    assert {seriallog.task_of(key) for key in found.tasks} == {"ws-client", "(none)", "httpd"}
    assert (found.unparsed, found.truncated) == (0, 0)
    assert len(found.mentioning("ws-client")) == 2 and not found.mentioning("mqtt_worker")


def test_a_bench_line_with_an_unregistered_task_parses_whole():
    found = seriallog.late_entries([BENCH_UNREGISTERED])
    assert found.tasks == {"mqtt_worker": [1, 220], "dali-sniff": [1, 251],
                           "0x4ff89254 (not registered)": [1, 217]}
    assert found.unparsed == 0


def test_a_line_another_log_cut_into_counts_the_items_it_lost():
    found = seriallog.late_entries([BENCH_INTERLEAVED])
    assert found.tasks == {"ota-run": [3, 330]}
    assert found.unparsed == 1 and found.mentioning("ipc0")


def test_a_line_at_the_firmware_report_limit_is_counted_truncated():
    items = ", ".join("task%02d×1 (max 200 us @0x40000000<0x40000000)" % n for n in range(20))
    text = items.encode()[:seriallog.LATE_REPORT_BYTES].decode("utf-8", "ignore")
    line = "W (1) x: DALI ISR late entries (delayed; see raw deficit for losses): " + text
    assert seriallog.late_entries([line]).truncated == 1
    assert seriallog.late_entries([BENCH_UNREGISTERED]).truncated == 0


def test_the_soak_reads_late_entries_from_its_offset_through_the_toolkit(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(LATE + "\n")
    start = log.stat().st_size
    with log.open("a") as fh:
        fh.write(LATE.replace("ws-client×2", "mqtt_task×3") + "\n")
    assert SOAK.late_entries(log, start) == {"mqtt_task": [3, 140], "(none)": [1, 9],
                                             "httpd+isr": [1, 60], "ws-client+isr": [1, 300]}
    assert SOAK.late_entries(tmp_path / "missing.log", 0) == {}


def test_the_provocation_rewrites_the_name_it_found_and_restores_it_exactly():
    registry = _Registry({"name": "Kitchen", "notes": "the owner's note"})
    rewrite = provoke.NameRewrite(registry, 2)
    for _ in range(3):
        rewrite()
    assert registry.patches == [{"name": "Kitchen"}] * 3 and rewrite.writes == 3
    registry.record["name"] = "renamed meanwhile"
    assert rewrite.restore()
    assert registry.record == {"name": "Kitchen", "notes": "the owner's note"}


def test_the_provocation_writes_only_registered_gear_the_bench_may_write():
    assert provoke.first_registered(_Unfiltered([9, 1, 4]), {4, 9}) == 4
    assert provoke.first_registered(_Unfiltered([9, 1, 4]), {7}) is None
    assert provoke.first_registered(_Unfiltered([]), {0, 2, 3}) is None


class _Unfiltered:
    def __init__(self, shorts):
        self.shorts = shorts

    def devices_unfiltered(self):
        return {"physical_devices": [{"short_address": s} for s in self.shorts]}


HEARTBEAT = "2026-09-29T10:%02d:00.000Z I (%d) dali2rust: firmware heartbeat: uptime=%ds"


def _beats(*uptimes):
    return [HEARTBEAT % (n, uptime * 1000, uptime) for n, uptime in enumerate(uptimes)]


def test_the_heartbeat_run_counts_only_consecutive_minutes():
    assert seriallog.heartbeat_run(_beats(*range(60, 660, 60))) == 10
    assert seriallog.heartbeat_run(_beats(60, 120, 240, 300, 360)) == 3
    assert seriallog.heartbeat_run(_beats(600, 60, 120)) == 2
    assert seriallog.heartbeat_run(["noise"]) == 0


def test_the_log_is_known_to_have_passed_a_moment_by_the_firmware_stamp():
    lines = ["2026-09-29T10:00:00.000Z I (4999) x: HTTP access: GET /api/v1/health",
             "2026-09-29T10:00:01.000Z unstamped line"]
    assert seriallog.dut_ms(lines[0]) == 4999 and seriallog.dut_ms(lines[1]) is None
    assert seriallog.logged_past(lines, 4999) and not seriallog.logged_past(lines, 5000)
    assert seriallog.newest_ms(lines + ["x W (12) y"]) == 4999
    assert seriallog.newest_ms(lines[1:]) is None and not seriallog.logged_past([], 0)


BOOTED_S = datetime.datetime(2026, 9, 30, 9, tzinfo=datetime.timezone.utc).timestamp()
MINUTE_S, HOUR_S, DAY_S = 60, 3600, 86400
U32_WRAP_MS = 1 << 32
RAW_PHY = ("dali2rust_adapters: DALI PHY interrupt: level %d, cpu int 17, core 0, source TG0_T0, "
           "raw handler")
DRIVER_PHY = "dali2rust_adapters: DALI PHY interrupt: level 3, gptimer driver handler"
BEAT = "dali2rust: firmware heartbeat: uptime=1s"
PHY_AT_MS, BEAT_AT_MS = 900, 60000
DRIFT_S = 100
CRASHED_AT_S = 30
SMALL_BLOCK = 7


def _logged(booted_s, uptime_ms, text, late_s=0.0):
    return "%s I (%d) %s" % (serialmon.stamp(booted_s + uptime_ms / 1000 + late_s),
                             uptime_ms % U32_WRAP_MS, text)


def _rom(at_s):
    return "%s ESP-ROM:esp32p4-eco2-20240710" % serialmon.stamp(at_s)


def _level(*oldest_first):
    return seriallog.running_boot_phy_level(list(reversed(oldest_first)))


def test_the_running_boot_names_the_phy_level_it_logged():
    later = BOOTED_S + HOUR_S
    assert _level(_rom(BOOTED_S), _logged(BOOTED_S, PHY_AT_MS, DRIVER_PHY),
                  _logged(BOOTED_S, BEAT_AT_MS, BEAT)) == 3
    assert _level(_logged(BOOTED_S, PHY_AT_MS, RAW_PHY % 5), _logged(BOOTED_S, BEAT_AT_MS, BEAT),
                  _rom(later), _logged(later, PHY_AT_MS, DRIVER_PHY),
                  _logged(later, BEAT_AT_MS, BEAT)) == 3
    assert _level(_logged(BOOTED_S, PHY_AT_MS, DRIVER_PHY),
                  _logged(BOOTED_S, 30 * DAY_S * 1000, BEAT, late_s=DRIFT_S)) == 3


def test_a_boot_whose_phy_line_the_log_missed_takes_no_level_from_an_older_boot():
    later = BOOTED_S + CRASHED_AT_S + MINUTE_S
    assert _level(_logged(BOOTED_S, PHY_AT_MS, DRIVER_PHY),
                  _logged(BOOTED_S, CRASHED_AT_S * 1000, BEAT),
                  _logged(later, BEAT_AT_MS, BEAT)) is None
    assert _level(_logged(BOOTED_S, PHY_AT_MS, DRIVER_PHY), _rom(later),
                  _logged(later, BEAT_AT_MS, BEAT)) is None
    assert _level(_logged(BOOTED_S, BEAT_AT_MS, BEAT)) is None


def test_a_wrapped_firmware_stamp_hides_the_boot_start():
    assert _level(_logged(BOOTED_S, PHY_AT_MS, DRIVER_PHY),
                  _logged(BOOTED_S, U32_WRAP_MS + BEAT_AT_MS, BEAT)) is None


def test_the_log_is_read_newest_first_across_blocks(tmp_path):
    text = "\n".join(["first", "", "ünïcode line", "last", ""])
    path = tmp_path / "serial.log"
    path.write_text(text)
    assert list(seriallog.lines_newest_first(path, SMALL_BLOCK)) == list(reversed(text.split("\n")))
    serial = seriallog.SerialLog(_cfg(tmp_path))
    assert serial.boot_phy_level() is None
    serial.log_path.parent.mkdir(parents=True)
    serial.log_path.write_text("\n".join([_logged(BOOTED_S, PHY_AT_MS, RAW_PHY % 5),
                                          _logged(BOOTED_S, BEAT_AT_MS, BEAT)]) + "\n")
    assert serial.boot_phy_level() == 5
