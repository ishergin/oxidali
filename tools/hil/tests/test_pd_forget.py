import pytest

from hil.wait import wait_until

FORGET_SHORT = 3

RESCAN_MODE = "scan_known_short_addresses"

GONE_BUDGET_S = 5.0

RESTORE_BUDGET_S = 90.0


def _in_list(api, short):
    return any(d["short_address"] == short
               for d in api.devices().get("physical_devices", []))


@pytest.fixture
def forget_guard(api):
    if not _in_list(api, FORGET_SHORT):
        pytest.skip("SA%d, the forget target, is not on this segment"
                    % FORGET_SHORT)
    before = api.state(FORGET_SHORT)
    config = api.config_snapshot()
    try:
        yield before
    finally:
        if not _in_list(api, FORGET_SHORT):
            api.discovery(RESCAN_MODE)
            wait_until(lambda: _in_list(api, FORGET_SHORT), RESTORE_BUDGET_S,
                       interval_s=2.0, desc="rescan restores the forgotten device")
        if _in_list(api, FORGET_SHORT):
            api.device_patch(FORGET_SHORT, {
                "name": before.get("name") or "",
                "notes": before.get("notes") or "",
            })
        api.config_restore(config)


@pytest.mark.destructive
def test_forgetting_a_device_drops_it_and_a_scan_brings_it_back_blank(api, forget_guard):
    api.device_patch(FORGET_SHORT, {"notes": "hil-forget-marker"})
    assert api.state(FORGET_SHORT).get("notes") == "hil-forget-marker"

    api.device_forget(FORGET_SHORT)
    assert wait_until(lambda: not _in_list(api, FORGET_SHORT), GONE_BUDGET_S,
                      interval_s=0.5, desc="device leaves the list"), \
        "the record was still listed after a 204"

    api.discovery(RESCAN_MODE)
    assert wait_until(lambda: _in_list(api, FORGET_SHORT), RESTORE_BUDGET_S,
                      interval_s=2.0, desc="scan re-creates the device"), \
        "a scan did not bring the device back — it is still on the wire"

    back = api.state(FORGET_SHORT)
    assert not (back.get("notes") or ""), \
        "the record came back WITH its notes: the delete never reached the slice"
