import pytest

from hil.lamp_guard import spell
from hil.wait import wait_until
from hil_test_guards import ask_guard

RESCAN_MODE = "scan_known_short_addresses"

GONE_BUDGET_S = 5.0

RESTORE_BUDGET_S = 90.0


def _in_list(api, short):
    return any(d["short_address"] == short
               for d in api.devices().get("physical_devices", []))


def _record(api, short):
    return "adapters/%d/physical-devices/%d" % (api.adapter, short)


def forget_target(api):
    present = set(api.present_addrs())
    targets = [short for short in api.lamp_addrs() if short in present]
    if not targets:
        pytest.skip("no present lamp of HIL_LAMP_SHORTS=%s to forget"
                    % spell(api.cfg.lamp_short_set()))
    ask_guard(api, "DELETE", _record(api, targets[0]))
    return targets[0]


@pytest.fixture
def forget_guard(api):
    short = forget_target(api)
    before = api.state(short)
    config = api.config_snapshot()
    try:
        yield short
    finally:
        if not _in_list(api, short):
            api.discovery(RESCAN_MODE)
            wait_until(lambda: _in_list(api, short), RESTORE_BUDGET_S,
                       interval_s=2.0, desc="rescan restores the forgotten device")
        if _in_list(api, short):
            api.device_patch(short, {
                "name": before.get("name") or "",
                "notes": before.get("notes") or "",
            })
        api.config_restore(config)


@pytest.mark.destructive
def test_forgetting_a_device_drops_it_and_a_scan_brings_it_back_blank(api, forget_guard):
    short = forget_guard
    api.device_patch(short, {"notes": "hil-forget-marker"})
    assert api.state(short).get("notes") == "hil-forget-marker"

    api.device_forget(short)
    assert wait_until(lambda: not _in_list(api, short), GONE_BUDGET_S,
                      interval_s=0.5, desc="device leaves the list"), \
        "the record was still listed after a 204"

    api.discovery(RESCAN_MODE)
    assert wait_until(lambda: _in_list(api, short), RESTORE_BUDGET_S,
                      interval_s=2.0, desc="scan re-creates the device"), \
        "a scan did not bring the device back — it is still on the wire"

    back = api.state(short)
    assert not (back.get("notes") or ""), \
        "the record came back WITH its notes: the delete never reached the slice"
