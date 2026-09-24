import os
import time

import pytest

ARM_TIMEOUT_S = 120
RELEASE_TIMEOUT_S = 120
POWER_DOWN_MS = 45
SYSTEM_FAILURE_MS = 550
POLL_SLACK_MS = 1500


def _wire(api):
    return api.diagnostics()["dali_wire"]


def _await(api, predicate, timeout_s, what):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        wire = _wire(api)
        if predicate(wire):
            return wire, time.monotonic()
        time.sleep(0.05)
    pytest.fail("%s did not happen within %d s; last wire block: %r"
                % (what, timeout_s, _wire(api)))


@pytest.mark.destructive
def test_a_shorted_bus_is_classified_by_the_isr_and_recovers(api, test_artifacts):
    if os.environ.get("HIL_BUS_SHORT") != "1":
        pytest.skip(
            "F15 needs a person to short the DALI bus — set HIL_BUS_SHORT=1 and "
            "run this test alone when you are at the rig")

    before = _wire(api)
    assert before["bus_power_down_active"] == 0, (
        "the bus already reads as down before the test: %r" % before)
    assert before["system_failure_active"] == 0

    print("\n>>> SHORT THE DALI BUS NOW (hold it for at least 2 s), "
          "then release it when this says so <<<")
    down, t_down = _await(api, lambda w: w["bus_power_down_active"] == 1,
                          ARM_TIMEOUT_S, "bus_power_down_active")
    failure, t_failure = _await(api, lambda w: w["system_failure_active"] == 1,
                                RELEASE_TIMEOUT_S, "system_failure_active")
    escalation_ms = (t_failure - t_down) * 1000.0

    print("\n>>> RELEASE THE SHORT NOW <<<")
    recovered, _ = _await(api, lambda w: w["bus_power_down_active"] == 0,
                          RELEASE_TIMEOUT_S, "bus_power_down_active clearing")
    _await(api, lambda w: w["system_failure_active"] == 0,
           RELEASE_TIMEOUT_S, "system_failure_active clearing")
    after = _wire(api)

    test_artifacts.attach_json("f15_wire", {
        "before": before, "during_down": down, "during_failure": failure,
        "after_release": recovered, "final": after,
        "escalation_ms": escalation_ms,
    })

    assert after["bus_power_down_entries"] == before["bus_power_down_entries"] + 1, (
        "power-down entries moved by %d, not 1 — the classifier chattered"
        % (after["bus_power_down_entries"] - before["bus_power_down_entries"]))
    assert after["system_failure_entries"] == before["system_failure_entries"] + 1, (
        "system-failure entries moved by %d, not 1"
        % (after["system_failure_entries"] - before["system_failure_entries"]))
    assert escalation_ms <= SYSTEM_FAILURE_MS + POLL_SLACK_MS, (
        "escalation to system failure took %.0f ms, past §4.11.3's %d ms plus "
        "the %d ms this polled surface costs" % (escalation_ms, SYSTEM_FAILURE_MS,
                                                 POLL_SLACK_MS))
    assert down["frames_sent_by_priority"] == failure["frames_sent_by_priority"], (
        "frames were sent while the bus was down: %r -> %r"
        % (down["frames_sent_by_priority"], failure["frames_sent_by_priority"]))
