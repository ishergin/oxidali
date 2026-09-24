import threading
import time

import pytest
from requests.exceptions import RequestException

pytestmark = pytest.mark.slow

SOAK_DURATION_S = 300
RING_LOSS_BUDGET = 8


@pytest.mark.hil_id("HIL-LOAD-01")
@pytest.mark.serial
@pytest.mark.sniffer
def test_sustained_mixed_load_no_reboot(api, hil_config, lamps, sniffer,
                                        serial_log, paced, state_snapshot,
                                        test_artifacts):
    from hil.foreign import ForeignMaster
    foreign = None
    try:
        foreign = ForeignMaster(hil_config)
        foreign.probe()
    except Exception:
        pass

    shorts = [lamps.by_label[label] for label in lamps.labels()]
    uptime_before = api.health().get("uptime_seconds", 0)
    counts = {"ts": 0, "query": 0, "foreign": 0, "reads": 0}
    stalls = []
    with serial_log.window() as serial_win, sniffer.window() as win:
        started = time.monotonic()
        i = 0
        while time.monotonic() - started < SOAK_DURATION_S:
            paced(1.0)
            short = shorts[i % len(shorts)]
            try:
                if i % 7 == 3:
                    api.cmd(short, 0xA0)
                    counts["query"] += 1
                elif foreign is not None and i % 11 == 5:
                    try:
                        foreign.dapc(short, 60 + (i % 150))
                        counts["foreign"] += 1
                    except Exception:
                        counts["foreign_errors"] = counts.get("foreign_errors", 0) + 1
                        if counts["foreign_errors"] >= 3:
                            foreign = None
                else:
                    api.ts(short, {"power": "on", "level": 40 + (i * 13) % 200})
                    counts["ts"] += 1
                if i % 10 == 0:
                    api.devices()
                    counts["reads"] += 1
            except RequestException as exc:
                stalls.append({"iteration": i,
                               "at_s": round(time.monotonic() - started, 1),
                               "error": type(exc).__name__,
                               "detail": str(exc)[:200]})
                break
            i += 1
        elapsed = time.monotonic() - started
        stats = win.stats()
        rebooted = serial_win.reboot_detected()
    try:
        health = api.health()
    except RequestException as exc:
        health = {"status": "unreachable",
                  "error": "%s: %s" % (type(exc).__name__, exc)}
    test_artifacts.attach_json("soak", {
        "counts": counts, "sniffer": stats, "health": health,
        "uptime_before": uptime_before, "foreign_used": foreign is not None,
        "elapsed_s": round(elapsed, 1), "dut_stalls": stalls})
    try:
        assert not rebooted, "DUT boot marker appeared during the soak"
        assert health.get("status") == "ok", health
        assert health.get("uptime_seconds", 0) >= uptime_before + elapsed - 30, \
            "uptime did not grow monotonically (hidden reboot?)"
        assert not stalls, (
            "DUT stopped answering under sustained load (no reboot, no boot "
            "marker): %s" % stalls)
        assert stats["counter_missed"] <= RING_LOSS_BUDGET, stats
    finally:
        try:
            api.off_all()
        except RequestException:
            pass


@pytest.mark.hil_id("HIL-LOAD-02")
def test_unpaced_burst_yields_honest_backpressure(api, lamps, state_snapshot,
                                                  test_artifacts):
    short = lamps.by_label[lamps.labels()[0]]
    results = []
    lock = threading.Lock()

    def fire(level):
        outcome = api.raw_request("POST", "dali/level",
                                  {"wire_address": short << 1, "level": level})
        with lock:
            results.append(outcome)

    threads = [threading.Thread(target=fire, args=(40 + i * 10,))
               for i in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    test_artifacts.attach_json("burst", results)
    statuses = {status for status, _ in results}
    assert statuses <= {200, 503}, statuses
    for status, body in results:
        if status == 503:
            assert body.get("error") in ("confirmation_slots_exhausted",
                                         "commands_ingress_overload",
                                         "execution_failed"), body
    time.sleep(1.5)
    resp = api.dapc(short, 90)
    assert resp.get("success") is True, resp
    api.off(short)


REQUIRED_DELIVERY_LOAD_S = 90


@pytest.mark.serial
def test_required_delivery_is_exercised_and_holds(api, serial_log, test_artifacts):
    def worker_counters():
        return api.diagnostics().get("dali_worker", {})

    before = worker_counters()
    stop = threading.Event()
    errors = []

    def hammer():
        while not stop.is_set():
            try:
                api.devices()
                api.diagnostics()
            except RequestException:
                pass
            except Exception as exc:
                errors.append(repr(exc))

    threads = [threading.Thread(target=hammer, daemon=True) for _ in range(3)]
    with serial_log.window() as serial_win:
        for t in threads:
            t.start()
        try:
            op = api.discovery("scan_known_short_addresses")
            view = api.wait_op(op, timeout_s=REQUIRED_DELIVERY_LOAD_S + 120)
        finally:
            stop.set()
            for t in threads:
                t.join(timeout=10)

    after = worker_counters()
    delta = {
        key: after.get(key, 0) - before.get(key, 0)
        for key in ("event_publish_retried", "event_publish_failed",
                    "event_publish_backoff_ms", "discovery_progress_events_published")
    }
    test_artifacts.attach_json("required_delivery", {
        "scan_status": view.get("status"),
        "delta": delta,
        "reader_errors": errors[:5],
    })

    assert view.get("status") == "succeeded", (
        "the scan itself failed, so nothing can be concluded about delivery: %r"
        % (view,)
    )
    assert not serial_win.reboot_detected(), (
        "the DUT rebooted during the run, so the counter deltas span a reset "
        "and mean nothing"
    )
    assert delta["event_publish_failed"] == 0, (
        "a required event was lost even with the backoff: %r. This is the "
        "ISSUE-50 shape — the fact has no second carrier." % (delta,)
    )
    if delta["event_publish_retried"] == 0:
        pytest.skip(
            "inconclusive: the events ingress was never full, so required "
            "delivery (ADR-021 Ф1) was not exercised (retried=0). Not a pass.\n"
            "\n"
            "Measured 2026-08-19 with the FULL fleet on the segment — 64 "
            "devices, scan succeeded in 44 s — and the counter still did not "
            "move. That is Ф2 working, not a rig shortfall: streaming the "
            "progress events one device at a time puts a full identity probe "
            "between consecutive publishes, so the queue drains faster than the "
            "scan fills it. The burst ISSUE-50 lost its tail to no longer "
            "exists on this path.\n"
            "\n"
            "So a realistic workload can no longer reach Ф1 here, and a bench "
            "run leaving this at 0 is a HEALTH signal rather than a coverage "
            "gap. The layer that does exercise it is "
            "dali2rust-bus/tests/publish_required.rs, which fills a real "
            "ingress on purpose. Making the bench reach it would mean building "
            "a saturator for a condition the product no longer creates — worth "
            "doing only if some other producer starts bursting."
        )
    assert delta["event_publish_backoff_ms"] < REQUIRED_DELIVERY_LOAD_S * 1000, (
        "the worker slept more than the run lasted: %r" % (delta,)
    )
