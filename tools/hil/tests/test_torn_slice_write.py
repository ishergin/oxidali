import os
import time

import pytest

CUT_TIMEOUT_S = 180
RETURN_TIMEOUT_S = 180
SECOND_CYCLE_TIMEOUT_S = 300
WRITE_INTERVAL_S = 0.7


def _flushes(api):
    return api.diagnostics()["persistence"]["flush_success_total"]


def _hydrate(api):
    p = api.diagnostics()["persistence"]
    return p["hydrate_loaded_total"], p["hydrate_default_total"], p["hydrate_error_total"]


def _wait_gone(api, timeout_s):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            api.health()
        except Exception:
            return True
        time.sleep(0.3)
    return False


def _wait_back(api, timeout_s):
    deadline = time.monotonic() + timeout_s
    with api.expect_reboot():
        while time.monotonic() < deadline:
            try:
                api.health()
                return True
            except Exception:
                time.sleep(1.0)
    return False


@pytest.mark.destructive
def test_a_torn_slice_write_comes_back_as_the_stored_revision(api, test_artifacts):
    if os.environ.get("HIL_POWER_CUT") != "1":
        pytest.skip(
            "ISSUE-56 needs a person to cut the controller's power mid-write — "
            "set HIL_POWER_CUT=1 and run this test alone when you are at the rig")

    short = api.addrs()[0]
    marker = "torn-%d" % int(time.time())
    api.device_patch(short, {"name": marker})
    baseline = _flushes(api)
    deadline = time.monotonic() + 30
    while _flushes(api) <= baseline and time.monotonic() < deadline:
        time.sleep(0.5)
    assert _flushes(api) > baseline, "no flush landed before the cut; nothing to tear"

    print("\n>>> CUT THE CONTROLLER'S POWER WHILE THE COUNTER BELOW IS MOVING <<<")
    cut_at = None
    deadline = time.monotonic() + CUT_TIMEOUT_S
    while time.monotonic() < deadline:
        try:
            api.device_patch(short, {"name": marker})
            print("  flushes: %d" % _flushes(api))
        except Exception:
            cut_at = time.monotonic()
            break
        time.sleep(WRITE_INTERVAL_S)
    assert cut_at, "power never went away — nothing was torn"

    assert _wait_gone(api, 30) or True
    print("\n>>> POWER IT BACK ON <<<")
    assert _wait_back(api, RETURN_TIMEOUT_S), "the controller did not come back"

    loaded_1, defaulted_1, errored_1 = _hydrate(api)
    name_1 = api.state(short).get("name")

    print("\n>>> CYCLE THE POWER ONCE MORE (off, then on) — %d s <<<" % SECOND_CYCLE_TIMEOUT_S)
    assert _wait_gone(api, SECOND_CYCLE_TIMEOUT_S), (
        "power did not go away for the second cycle within %d s"
        % SECOND_CYCLE_TIMEOUT_S)
    assert _wait_back(api, RETURN_TIMEOUT_S), "the controller did not come back the second time"

    loaded_2, defaulted_2, errored_2 = _hydrate(api)
    name_2 = api.state(short).get("name")

    test_artifacts.attach_json("issue56_torn_write", {
        "marker": marker,
        "first_boot": {"loaded": loaded_1, "defaulted": defaulted_1,
                       "errored": errored_1, "name": name_1},
        "second_boot": {"loaded": loaded_2, "defaulted": defaulted_2,
                        "errored": errored_2, "name": name_2},
    })

    assert loaded_1 > 0, "nothing hydrated after the torn write: %r" % (loaded_1,)
    assert loaded_2 > 0, "nothing hydrated on the second boot either"
    assert name_1 == marker, (
        "device %d came back as %r instead of the stored %r — the store "
        "re-defaulted, which every field-level check would have called healthy"
        % (short, name_1, marker))
    assert name_2 == marker, "the value did not survive the second power cycle"
    assert defaulted_1 == 0 and defaulted_2 == 0, (
        "slices hydrated from defaults: %d then %d" % (defaulted_1, defaulted_2))
    assert errored_1 == 0 and errored_2 == 0, (
        "hydrate errors: %d then %d — a torn slice that neither loaded nor fell "
        "back is the failure ISSUE-56 is about" % (errored_1, errored_2))
