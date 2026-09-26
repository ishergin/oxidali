import re
import types
from collections import Counter
from pathlib import Path

import pytest
import requests

from hil import api as api_mod
from hil import sniffer, validity
from hil.lamp_guard import LampGuard
from hil.oracle import CameraOracle



def test_bus_contended_is_recognised_and_a_real_failure_is_not():
    contended = {"error": {"code": "operation_failed", "message": "bus_contended"},
                 "operation_id": "pd-disc-0-203", "status": "failed",
                 "type": "discovery"}
    assert api_mod.op_contended(contended)

    assert not api_mod.op_contended(
        {"error": {"code": "operation_failed", "message": "adapter_disabled"},
         "status": "failed"})
    assert not api_mod.op_contended({"status": "succeeded"})
    assert not api_mod.op_contended({"status": "timed_out"})
    assert not api_mod.op_contended(None)



class _FakeResponse:
    def __init__(self, status, payload):
        self.status_code = status
        self.content = b"{}"
        self._payload = payload

    def json(self):
        return self._payload


def _client(monkeypatch, responses):
    cfg = types.SimpleNamespace(base="http://dut", adapter=0)
    client = api_mod.Client.__new__(api_mod.Client)
    client.cfg = cfg
    client.base = "http://dut"
    client.adapter = 0
    client.timeout_s = 1.0
    client.init_ledger()
    client._rebooting = False
    client.guard = LampGuard({0})
    queue = list(responses)

    class _Session:
        def request(self, method, url, json=None, timeout=None):
            if not queue:
                raise requests.RequestException("no more scripted responses")
            return _FakeResponse(*queue.pop(0))

        def close(self):
            pass

    client.http = _Session()
    monkeypatch.setattr(api_mod.time, "sleep", lambda *_: None)
    return client


def test_a_504_retry_is_counted_not_swallowed(monkeypatch):
    client = _client(monkeypatch, [(504, {"error": "timeout"}), (200, {"ok": True})])
    assert client._req("GET", "health") == {"ok": True}
    assert client.retries["http_504"] == 1


def test_a_503_retry_is_counted(monkeypatch):
    client = _client(monkeypatch, [(503, {"error": "busy"}), (200, {"ok": True})])
    assert client._req("GET", "health") == {"ok": True}
    assert client.retries["http_503"] == 1


def test_a_transport_retry_is_counted(monkeypatch):
    client = _client(monkeypatch, [(200, {"ok": True})])
    calls = {"n": 0}
    real = client.http.request

    def flaky(method, url, json=None, timeout=None):
        calls["n"] += 1
        if calls["n"] == 1:
            raise requests.ConnectionError("reset by peer")
        return real(method, url, json=json, timeout=timeout)

    client.http.request = flaky
    assert client._req("GET", "health") == {"ok": True}
    assert client.retries["http_transport"] == 1
    assert client.retries["http_reboot_race"] == 0, (
        "a reset on a live controller is the residual ISSUE-55 is about, not a "
        "reboot the test asked for"
    )


def test_a_reboot_race_is_counted_apart_from_a_transport_fault(monkeypatch):
    client = _client(monkeypatch, [(200, {"ok": True})])
    calls = {"n": 0}
    real = client.http.request

    def flaky(method, url, json=None, timeout=None):
        calls["n"] += 1
        if calls["n"] == 1:
            raise requests.ConnectionError("connection refused")
        return real(method, url, json=json, timeout=timeout)

    client.http.request = flaky
    with client.expect_reboot():
        assert client._req("GET", "health") == {"ok": True}
    assert client.retries["http_reboot_race"] == 1
    assert client.retries["http_transport"] == 0, (
        "a reboot the test asked for must not spend the residual's budget"
    )
    assert client._rebooting is False


def test_a_clean_exchange_counts_nothing(monkeypatch):
    client = _client(monkeypatch, [(200, {"ok": True})])
    client._req("GET", "health")
    assert not client.retries



def test_a_first_poll_404_keeps_waiting_instead_of_reporting_a_commit(monkeypatch):
    client = _client(monkeypatch, [
        (404, {"error": "not_found"}),
        (200, {"operation_id": "cfg-grp-0-7", "status": "succeeded"}),
    ])
    assert client.wait_op("cfg-grp-0-7")["status"] == "succeeded"


def test_an_operation_that_never_appears_is_not_a_commit(monkeypatch):
    client = _client(monkeypatch, [])
    client.http.request = lambda *a, **k: _FakeResponse(404, {"error": "not_found"})
    out = client.wait_op("cfg-grp-0-7", timeout_s=0.05)
    assert out["status"] == "never_registered"


def test_a_404_after_the_operation_was_seen_is_a_real_eviction(monkeypatch):
    client = _client(monkeypatch, [
        (200, {"operation_id": "cfg-grp-0-7", "status": "running"}),
        (404, {"error": "not_found"}),
    ])
    assert client.wait_op("cfg-grp-0-7")["status"] == "evicted_or_unknown"


def test_raw_counts_the_exchange_and_the_silence(monkeypatch):
    client = _client(monkeypatch, [
        (200, {"success": True, "backward_frame": 120}),
        (200, {"success": False, "error": "execution_failed", "backward_frame": 0}),
    ])
    client.raw(0x0164)
    client.raw(0x0164, expects_backward=True)
    assert client.retries["raw_exchanges"] == 2
    assert client.retries["raw_unanswered"] == 1



SLACK = 30.0


def test_a_reboot_mid_test_is_detected():
    assert validity.uptime_broke((1000.0, 1452.0), 1009.0, 3.0, SLACK)


def test_a_reboot_late_in_a_long_session_is_still_detected():
    prev = (0.0, 100.0)
    two_hours_later, fresh_boot = 7200.0, 300.0
    assert fresh_boot > prev[1], "the naive check would pass here"
    assert validity.uptime_broke(prev, two_hours_later, fresh_boot, SLACK)


def test_normal_growth_and_slack_do_not_trip_it():
    assert validity.uptime_broke((1000.0, 500.0), 1060.0, 560.0, SLACK) is None
    assert validity.uptime_broke((1000.0, 500.0), 1060.0, 535.0, SLACK) is None
    assert validity.uptime_broke((1000.0, 500.0), 1060.0, 520.0, SLACK)


def test_the_first_sample_anchors_instead_of_judging():
    assert validity.uptime_broke(None, 1.0, 1.0, SLACK) is None



BUDGET_TEXT = """
# comment
raw_unanswered   0
http_504         3
attr_read       -1
"""


def _budget(tmp_path):
    path = tmp_path / "retry_budget.txt"
    path.write_text(BUDGET_TEXT)
    return validity.load_budget(path)


def test_budget_parses_and_ignores_comments(tmp_path):
    assert _budget(tmp_path) == {"raw_unanswered": 0, "http_504": 3,
                                 "attr_read": validity.NOT_MEASURED}


def test_over_budget_is_a_breach_and_under_is_not(tmp_path):
    budget = _budget(tmp_path)
    assert validity.breaches({"http_504": 4}, budget) == [("http_504", 4, 3)]
    assert validity.breaches({"http_504": 3}, budget) == []
    assert validity.breaches({"raw_unanswered": 1}, budget) == \
        [("raw_unanswered", 1, 0)]


def test_an_unmeasured_counter_never_gates(tmp_path):
    budget = _budget(tmp_path)
    assert validity.breaches({"attr_read": 999}, budget) == []
    assert validity.breaches({"never_heard_of_it": 999}, budget) == []


def test_a_missing_budget_file_gates_nothing(tmp_path):
    assert validity.load_budget(tmp_path / "absent.txt") == {}
    assert validity.breaches({"raw_unanswered": 5}, {}) == []


COUNTED_KIND = re.compile(r'count_retry\(\s*"([a-z0-9_]+)",')
NEW_KINDS = frozenset({"rules_put_reconciled", "teardown_write", "group_membership_reread",
                       "group_apply_reconverge", "post_commission_requery",
                       sniffer.SNIFFER_RESEND})


def _silent_tap(tmp_path):
    tap = sniffer.SnifferTap.__new__(sniffer.SnifferTap)
    tap.retries, tap.retry_events, tap.witness_fallbacks = Counter(), [], 0
    tap.log_path = tmp_path / "sniffer.log"
    tap.log_path.write_text("")
    return tap


def test_every_repeated_step_reaches_the_ledger_and_its_budget(monkeypatch, tmp_path):
    client = _client(monkeypatch, [])
    client.count_retry("teardown_write", "group apply (ConnectionError)")
    tap, sent = _silent_tap(tmp_path), []
    with pytest.raises(AssertionError, match="no frame containing"):
        sniffer.Window(tap).expect_frame("DAPC short 2", timeout_s=0.01,
                                         resend=lambda: sent.append("DAPC"))
    oracle = CameraOracle(None, {"lamps": []}, None, None)
    oracle.count_retry("gear_colour_lag")
    foreign = types.SimpleNamespace(retries=Counter(foreign_master_silent=1))

    counters, events = validity.tally([client, tap, oracle, foreign])
    ledger = validity.collect(dict(counters, bus_contended=0))
    assert sent == ["DAPC"]
    assert ledger == {"teardown_write": 1, sniffer.SNIFFER_RESEND: 1,
                      "optical_gear_colour_lag": 1, "foreign_master_silent": 1}
    assert [row[0] for row in validity.breaches(ledger, validity.load_budget())] == \
        sorted(ledger)
    assert len(events) == 2 and "DAPC short 2" in events[1]


def test_every_kind_a_step_is_counted_under_has_a_shipped_budget_line():
    root = Path(validity.__file__).resolve().parent.parent
    sources = sorted(root.glob("hil/**/*.py")) + sorted(root.glob("tests/*.py"))
    kinds = {kind for path in sources for kind in COUNTED_KIND.findall(path.read_text())}
    budget = validity.load_budget()
    assert kinds >= NEW_KINDS - {sniffer.SNIFFER_RESEND}
    assert sorted((kinds | NEW_KINDS) - set(budget)) == []
    assert {kind: budget[kind] for kind in NEW_KINDS} == dict.fromkeys(NEW_KINDS, 0)



def test_report_names_a_reboot_and_the_over_budget_line(tmp_path):
    budget = _budget(tmp_path)
    lines = validity.format_report({
        "frame_server": "frame server pid 42, up 3.0h",
        "reboots": ["some_test: uptime went 1452s -> 3s across a 9s test"],
        "baseline": ["found the poller ENABLED — disabled it"],
        "ledger": {"http_504": 4, "raw_exchanges": 200, "raw_unanswered": 2},
    }, budget)
    body = "\n".join(lines)
    assert "pid 42" in body
    assert "BROKEN" in body and "uptime went" in body
    assert "poller ENABLED" in body
    assert "2 unanswered of 200 (1.0%)" in body
    assert "OVER BUDGET 3" in body


def test_report_is_calm_when_the_bench_is_clean(tmp_path):
    lines = validity.format_report({"frame_server": "no frame server running"},
                                   _budget(tmp_path))
    body = "\n".join(lines)
    assert "DUT uptime: continuous" in body
    assert "bench baseline: as expected" in body
    assert "retries: none recorded" in body


def test_report_carries_wb_and_the_observed_baseline(tmp_path):
    lines = validity.format_report({
        "observed": "poller off, 0/3 HCL schedules enabled, tz MSK-3",
        "wb": "reachable, tap live (root@192.168.11.110)",
    }, _budget(tmp_path))
    body = "\n".join(lines)
    assert "poller off, 0/3 HCL schedules enabled, tz MSK-3" in body
    assert "WB sniffer: reachable, tap live" in body


def test_wb_says_so_when_the_run_never_touched_it(tmp_path):
    body = "\n".join(validity.format_report({}, _budget(tmp_path)))
    assert "WB sniffer: not exercised by this run" in body



CENSUS = [
    "I (66512) dali2rust: task stack hwm (B free): main=6000 registry_worker=3000 httpd=26640 ",
    "I (126512) dali2rust: task stack hwm (B free): main=5900 registry_worker=1460 httpd=26000 ",
    "I (186512) dali2rust: firmware heartbeat: uptime=186s",
]


def _stack_budget(tmp_path):
    path = tmp_path / "stack_budget.txt"
    path.write_text(
        "# comment\n"
        "registry_worker      10240   8780\n"
        "httpd                36864   10224\n"
        "dali_worker          12288   -1\n")
    return validity.load_stack_budget(path)


def test_census_takes_the_minimum_across_the_whole_run():
    assert validity.stack_min_free(CENSUS) == {
        "main": 5900, "registry_worker": 1460, "httpd": 26000}


def test_census_ignores_lines_that_are_not_the_census():
    assert validity.stack_min_free(["I (1) x: firmware heartbeat: uptime=1s"]) == {}


def test_a_task_deeper_than_its_budget_is_a_breach(tmp_path):
    budget = _stack_budget(tmp_path)
    assert validity.stack_breaches(validity.stack_min_free(CENSUS), budget) == [
        ("httpd", 10864, 10224)]


def test_an_unmeasured_task_never_gates(tmp_path):
    budget = _stack_budget(tmp_path)
    deep = {"dali_worker": 8}
    assert validity.stack_breaches(deep, budget) == []


def test_a_run_with_no_census_says_so_instead_of_looking_clean(tmp_path):
    body = "\n".join(validity.format_report(
        {"stack_budget": _stack_budget(tmp_path)}, _budget(tmp_path)))
    assert "NO per-task census in the serial log" in body
    assert "monitor down" not in body


def test_the_census_is_read_with_and_without_the_slot_count():
    slots = ["I (66512) x: task stack hwm (B free, 31 of 64 slots): main=6000 "
             "httpd=26640 "]
    assert validity.stack_min_free(slots) == {"main": 6000, "httpd": 26640}
    tasks = ["I (66512) x: task stack hwm (B free, 33 tasks): main=6000 "
             "httpd=26640 "]
    assert validity.stack_min_free(tasks) == {"main": 6000, "httpd": 26640}
    assert validity.stack_min_free(CENSUS)["main"] == 5900


def test_callback_owned_stack_samples_require_freshness():
    lines = [
        "I (1) x: task stack hwm (B free, 2 tasks): main=6000 | observed: "
        "httpd=10352@0x1234@20:fresh mqtt_task=9000@0x5678@130000:stale"
    ]
    assert validity.stack_min_free(lines) == {"main": 6000, "httpd": 10352}
    assert validity.stack_observation_gaps(lines) == []
    missing = [
        "I (1) x: task stack hwm (B free, 1 tasks): main=6000 | observed: "
        "httpd=missing mqtt_task=missing"
    ]
    assert validity.stack_observation_gaps(missing) == ["httpd"]


def test_an_empty_census_body_is_not_a_census():
    assert validity.stack_min_free(["I (1) x: task stack hwm (B free): "]) == {}


def test_the_report_shows_used_against_the_stack(tmp_path):
    body = "\n".join(validity.format_report({
        "stack_min_free": validity.stack_min_free(CENSUS),
        "stack_budget": _stack_budget(tmp_path),
    }, _budget(tmp_path)))
    assert "stack headroom" in body
    assert "registry_worker" in body and "8780 used of 10240" in body
    assert "OVER BUDGET 10224" in body
    assert "unmeasured" not in body or "dali_worker" not in body


CENSUS_0_1_1101 = (
    "2026-09-24T14:08:22.418 I (185064) dali2rust_firmware::composition: task stack "
    "hwm (B free, 34 tasks): log-uart=1984 dali-sniff=5548 bus_task=6196 "
    "dali_worker=5840 registry_worker=7620 mqtt_worker=4760 ota-worker=1836 "
    "arb-supervisor=1524 operation_tracker_worker=6668 apply_orchestrator=6648 "
    "fanout_projector_worker=5620 sniffer_translator_worker=6800 hcl-scheduler=5416 "
    "poller=5992 rules-worker=8064 arbitration=2364 replication=6536 "
    "confirm-bridge=7088 display_worker=4544 ws_worker=5916 ip-watch=6616 "
    "census=1504 ws-client=6932 ws-client=6988 IDLE0=3620 IDLE1=3668 ipc0=668 ipc1=660 esp_timer=3660 "
    "Tmr Svc=1652 tiT=2564 sys_evt=2860 | observed: "
    "httpd=14324@0x4ff5b22c@522:fresh mqtt_task=2980@0x4ff278b0@4129:fresh "
)


def test_a_budget_line_no_task_matches_is_named(tmp_path):
    path = tmp_path / "stack_budget.txt"
    path.write_text("apply_orchestra      8192    5472\n"
                    "registry_worker      10240   8780\n")
    budget = validity.load_stack_budget(path)
    census = ["I (1) x: task stack hwm (B free, 2 tasks): "
              "apply_orchestrator=6648 registry_worker=7620 "]
    min_free = validity.stack_min_free(census)
    assert validity.stack_budget_dead_lines(min_free, budget) == ["apply_orchestra"]
    body = "\n".join(validity.format_report(
        {"stack_min_free": min_free, "stack_budget": budget}, _budget(tmp_path)))
    orphan = [line for line in body.splitlines() if "apply_orchestra " in line]
    assert len(orphan) == 1
    assert "matches no task in the census" in orphan[0]
    assert "gates nothing" in orphan[0]


def test_an_on_demand_task_is_reported_but_never_gates(tmp_path):
    path = tmp_path / "stack_budget.txt"
    path.write_text("ws-client            8192    1340\n"
                    "registry_worker      10240   8780\n")
    budget = validity.load_stack_budget(path)
    min_free = {"registry_worker": 7620}
    assert validity.stack_budget_orphans(min_free, budget) == ["ws-client"]
    assert validity.stack_budget_dead_lines(min_free, budget) == []
    body = "\n".join(validity.format_report(
        {"stack_min_free": min_free, "stack_budget": budget}, _budget(tmp_path)))
    assert "spawned on demand" in body
    assert "gates nothing" not in body


def test_an_httpd_missing_from_the_census_is_left_to_its_freshness_gate(tmp_path):
    budget = _stack_budget(tmp_path)
    lines = ["I (1) x: task stack hwm (B free, 2 tasks): registry_worker=3000 "
             "dali_worker=2000 | observed: httpd=10352@0x1234@400000:stale "]
    min_free = validity.stack_min_free(lines)
    assert "httpd" not in min_free
    assert validity.stack_budget_orphans(min_free, budget) == []
    assert validity.stack_observation_gaps(lines) == ["httpd"]


def test_a_run_with_no_census_names_no_orphan(tmp_path):
    assert validity.stack_budget_orphans({}, _stack_budget(tmp_path)) == []


def test_every_shipped_budget_line_is_spelled_as_the_census_prints_it():
    budget = validity.load_stack_budget()
    min_free = validity.stack_min_free([CENSUS_0_1_1101])
    assert validity.stack_budget_orphans(min_free, budget) == []
    no_client = re.sub(r" ws-client=\d+", "", CENSUS_0_1_1101)
    min_free = validity.stack_min_free([no_client])
    assert validity.stack_budget_orphans(min_free, budget) == ["ws-client"]
    assert validity.stack_budget_dead_lines(min_free, budget) == []


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__]))



BOOT_LADDER_LINES = [
    "2026-09-24T12:08:33.419 W (4664) dali2rust_adapters::runtime::registry_init:"
    " boot heap [composition start]: internal free=443343 largest=368640",
    "2026-09-24T12:08:33.419 W (4709) dali2rust_adapters::runtime::registry_init:"
    " boot heap [after hydrate]: internal free=420239 largest=344064",
    "2026-09-24T12:08:33.419 W (4768) dali2rust_adapters::runtime::registry_init:"
    " boot heap [after workers]: internal free=380415 largest=303104",
    "2026-09-24T12:08:34.422 W (4810) dali2rust_adapters::http::esp_idf:"
    " HTTP: internal heap before httpd: free=357199 B, largest_block=278528 B",
]


def test_the_ladder_is_read_from_real_log_lines_including_the_httpd_wording():
    ladder = validity.boot_heap_ladder(BOOT_LADDER_LINES)
    assert ladder["composition_start"] == 443343
    assert ladder["after_hydrate"] == 420239
    assert ladder["after_workers"] == 380415
    assert ladder["before_httpd"] == 357199


def test_two_boots_report_the_worse_one():
    worse = BOOT_LADDER_LINES[2].replace("free=380415", "free=349000")
    ladder = validity.boot_heap_ladder(BOOT_LADDER_LINES + [worse])
    assert ladder["after_workers"] == 349000


def test_below_the_floor_is_a_breach_and_above_it_is_not():
    ladder = validity.boot_heap_ladder(BOOT_LADDER_LINES)
    assert validity.boot_heap_breaches(ladder, {"after_workers": 360000}) == []
    assert validity.boot_heap_breaches(ladder, {"after_workers": 390000}) == [
        ("after_workers", 380415, 390000)
    ]


def test_a_stage_absent_from_the_log_gates_nothing():
    assert validity.boot_heap_breaches({}, {"after_workers": 180000}) == []


def test_a_missing_boot_heap_budget_file_gates_nothing(tmp_path):
    assert validity.load_boot_heap_budget(tmp_path / "nope.txt") == {}


def test_the_shipped_floors_parse_and_are_met_by_the_measured_ladder():
    budget = validity.load_boot_heap_budget()
    assert budget, "boot_heap_budget.txt must parse"
    assert validity.boot_heap_breaches(validity.boot_heap_ladder(BOOT_LADDER_LINES),
                                       budget) == []


def test_silence_is_reported_as_silence_not_as_health():
    said = " ".join(validity._boot_heap_lines({}))
    assert "not measured" in said



RUNTIME_STATS = {"controller": {
    "internal_min_free_bytes": 11135, "internal_free_bytes": 52571,
    "rust_internal_live_bytes": 40000, "rust_internal_peak_bytes": 61000,
    "rust_psram_peak_bytes": 90000, "free_heap_bytes": 27777931}}


def test_runtime_heap_is_read_from_stats_and_a_host_null_is_not_a_zero():
    figures = validity.runtime_heap_figures(RUNTIME_STATS)
    assert figures["internal_min_free"] == 11135
    assert figures["rust_internal_peak"] == 61000
    host = validity.runtime_heap_figures({"controller": {"internal_min_free_bytes": None}})
    assert host == {}


def test_an_unmeasured_runtime_floor_reports_and_never_gates():
    figures = validity.runtime_heap_figures(RUNTIME_STATS)
    assert validity.runtime_heap_breaches(figures, {"internal_min_free": -1}) == []
    said = " ".join(validity._runtime_heap_lines(
        {"runtime_heap": figures, "runtime_heap_budget": {"internal_min_free": -1}}))
    assert "unmeasured" in said


def test_a_runtime_minimum_under_its_floor_is_a_breach():
    figures = validity.runtime_heap_figures(RUNTIME_STATS)
    assert validity.runtime_heap_breaches(figures, {"internal_min_free": 8192}) == []
    assert validity.runtime_heap_breaches(figures, {"internal_min_free": 16384}) == [
        ("internal_min_free", 11135, 16384)
    ]


def test_the_shipped_runtime_floor_file_parses():
    assert "internal_min_free" in validity.load_runtime_heap_budget()


def test_no_runtime_reading_says_so():
    assert "not measured" in " ".join(validity._runtime_heap_lines({}))


def test_isr_timing_is_read_from_stats_and_an_absence_is_not_zeros():
    stats = {
        "dali": {
            "isr_ticks_lost_total": 0,
            "isr_ticks_extra_total": 0,
            "isr_late_ticks_total": 55,
            "isr_max_gap_us": 326,
        }
    }
    got = validity.isr_timing_counters(stats)
    assert got == {
        "isr_ticks_lost": 0,
        "isr_ticks_extra": 0,
        "isr_late_ticks": 55,
        "isr_max_gap_us": 326,
    }
    assert validity.isr_timing_counters({}) == {}
    assert validity.isr_timing_counters(None) == {}


def test_a_quiet_broker_does_not_redden_a_run_but_is_still_reported():
    lines = [
        "task stack hwm (B free, 3 tasks): dali_worker=2000 | observed: "
        "httpd=10352@0x3ffb1234@900:fresh mqtt_task=1800@0x3ffb9999@400000:stale ",
    ]
    assert validity.stack_observation_gaps(lines) == []
    states = validity.observed_task_states(lines)
    assert states["httpd"] == {"fresh"}
    assert states["mqtt_task"] == {"stale"}
    assert "mqtt_task" not in validity.GATED_OBSERVED_TASKS


def test_an_httpd_that_is_never_fresh_reddens_the_run():
    lines = [
        "task stack hwm (B free, 2 tasks): dali_worker=2000 | observed: "
        "httpd=10352@0x3ffb1234@400000:stale mqtt_task=missing ",
    ]
    assert validity.stack_observation_gaps(lines) == ["httpd"]


def test_a_run_with_no_observed_section_gates_nothing_and_claims_nothing():
    assert validity.stack_observation_gaps(
        ["task stack hwm (B free, 1 tasks): dali_worker=2000 "]) == []
    assert validity.observed_task_states([]) == {}


def test_a_boot_time_sink_failure_is_read_absolutely_not_as_a_delta():
    stats = {"dali": {"console_log_unavailable_total": 1}}
    assert validity.absolute_counters(stats) == {"console_log_unavailable": 1}
    assert "console_log_unavailable" not in validity.isr_timing_counters(stats)
    assert validity.absolute_counters({}) == {}
    assert validity.absolute_counters(None) == {}


def test_console_losses_are_reported_but_not_gated():
    stats = {
        "dali": {
            "console_log_dropped_total": 4,
            "console_log_busy_total": 1,
            "console_log_truncated_total": 0,
            "console_uart_errors_total": 0,
        }
    }
    got = validity.isr_timing_counters(stats)
    assert got["console_log_dropped"] == 4
    assert got["console_log_busy"] == 1
    assert got["console_log_busy"] <= got["console_log_dropped"]
    for name in ("console_log_dropped", "console_log_busy",
                 "console_log_truncated", "console_uart_errors"):
        assert name in validity.UNGATED_BUS_COUNTERS


def test_isr_totals_are_rebased_to_the_session_start():
    current = {"isr_ticks_lost": 12, "isr_ticks_extra": 8, "isr_max_gap_us": 520}
    baseline = {"isr_ticks_lost": 10, "isr_ticks_extra": 8, "isr_max_gap_us": 900}
    assert validity.counter_deltas(current, baseline) == {
        "isr_ticks_lost": 2,
        "isr_ticks_extra": 0,
        "isr_max_gap_us": 520,
    }


def test_isr_totals_rebase_across_a_u32_counter_wrap():
    assert validity.counter_deltas({"isr_ticks_lost": 3}, {"isr_ticks_lost": 0xFFFF_FFFE}) == {
        "isr_ticks_lost": 5,
    }


def test_isr_totals_stay_absent_without_a_same_boot_baseline():
    current = {"isr_ticks_lost": 3, "isr_max_gap_us": 520}
    assert validity.counter_deltas(current, {}) == {}
    assert validity.counter_deltas(current, {"isr_ticks_lost": 8}, same_boot=False) == {}


def test_isr_baseline_rejects_an_early_reboot_even_after_uptime_overtakes_start():
    assert not validity.same_boot_interval(
        1_000.0, 600_000, 4_600.0, 3_500_000, slack_s=30.0)


def test_isr_baseline_accepts_clocks_that_advance_together():
    assert validity.same_boot_interval(
        1_000.0, 600_000, 4_600.0, 4_199_500, slack_s=30.0)


def test_late_ticks_are_reported_and_not_gated():
    assert "isr_late_ticks" in validity.UNGATED_BUS_COUNTERS
    assert "isr_max_gap_us" in validity.UNGATED_BUS_COUNTERS
    assert "isr_ticks_lost" not in validity.UNGATED_BUS_COUNTERS
    assert "isr_ticks_extra" not in validity.UNGATED_BUS_COUNTERS
    budget = validity.load_budget()
    assert budget.get("isr_ticks_lost") == 0
    assert budget.get("isr_ticks_extra") == 0


def test_session_rates_round_up_and_need_exposure():
    deltas = {"isr_ticks_lost": 1, "answer_stage_late": 0, "frames": 9}
    hour = validity.session_rates(deltas, 3_600_000)
    assert hour == {"isr_ticks_lost_per_h": 1, "answer_stage_late_per_h": 0}
    ten = validity.session_rates(deltas, validity.MIN_RATE_EXPOSURE_MS)
    assert ten["isr_ticks_lost_per_h"] == 6
    assert validity.session_rates(deltas, 120_000) == {}
    assert validity.session_rates(deltas, None) == {}


def test_task_latency_gauges_are_values_not_deltas():
    current = {"answer_stage_max_ticks": 91, "persist_flush_max_ms": 546,
               "sniff_poll_gap_max_us": 168_000, "persist_flush_slow": 80}
    baseline = {"answer_stage_max_ticks": 40, "persist_flush_max_ms": 300,
                "sniff_poll_gap_max_us": 9_000, "persist_flush_slow": 2}
    out = validity.counter_deltas(current, baseline)
    assert out["answer_stage_max_ticks"] == 91
    assert out["persist_flush_max_ms"] == 546
    assert out["sniff_poll_gap_max_us"] == 168_000
    assert out["persist_flush_slow"] == 78


def test_task_latency_counters_are_read_from_stats():
    stats = {"dali": {"answer_staged_total": 30, "answer_stage_late_total": 1,
                      "persist_flush_slow_total": 78, "persist_gate_waits_total": 5}}
    out = validity.isr_timing_counters(stats)
    assert out["answer_staged"] == 30
    assert out["answer_stage_late"] == 1
    assert out["persist_flush_slow"] == 78
    assert out["persist_gate_waits"] == 5
