import time

import pytest

from hil.api import ApiError

DISCOVERY_SCAN_TIMEOUT_S = 180
QUERY_ACTUAL_LEVEL = 0xA0
REQUERY_ATTEMPTS = 3
REQUERY_PAUSE_S = 2.0

pytestmark = [pytest.mark.destructive, pytest.mark.sniffer]


@pytest.mark.hil_id("HIL-DSC-03")
def test_refresh_known_keeps_addresses(api, sniffer, test_artifacts):
    before = api.addrs()
    try:
        op = api.discovery("refresh_known")
    except ApiError as exc:
        pytest.skip("refresh_known unsupported on this firmware: %s" % exc)
    with sniffer.window() as win:
        view = api.wait_op(op, timeout_s=120)
        test_artifacts.attach_json("refresh_op", view)
        time.sleep(2.0)
        frames = win.frames()
    readdress = [f["decoded"] for f in frames
                 if "PROGRAM SHORT ADDRESS" in f["decoded"]
                 or ("RANDOMISE" in f["decoded"]
                     and f["decoded"].endswith("0x00"))]
    assert not readdress, readdress
    assert api.addrs() == before


@pytest.mark.hil_id("HIL-DSC-04")
def test_random_addresses_stable_across_scans(api, test_artifacts):
    maps = []
    for _ in range(3):
        view = api.wait_op(api.discovery("scan_known_short_addresses"),
                           timeout_s=DISCOVERY_SCAN_TIMEOUT_S)
        assert view.get("status") in ("succeeded", "failed"), view
        maps.append({d["short_address"]: d.get("random_address")
                     for d in api.devices()["physical_devices"]})
        time.sleep(1.0)
    flips = {}
    for short in maps[0]:
        values = {m.get(short) for m in maps}
        if len(values) > 1:
            flips[short] = sorted(v for v in values if v is not None)
    test_artifacts.attach_json("random_address_maps",
                               {"maps": maps, "flips": flips})
    assert not flips, (
        "random addresses changed across three scans of known short addresses: %s — "
        "a scan reads what the gear holds, so a flip is a backward frame attributed "
        "to the wrong query" % flips)


@pytest.mark.hil_id("HIL-DSC-02")
def test_commission_unaddressed_cycle_is_safe(api, sniffer, lamps,
                                              state_snapshot, test_artifacts):
    before = api.addrs()
    with sniffer.window() as win:
        op = api.wait_op(api.discovery("commission_unaddressed"),
                         timeout_s=180)
        test_artifacts.attach_json("commission_op", op)
        assert op.get("status") == "succeeded", op
        time.sleep(2.0)
        frames = [f["decoded"] for f in win.frames()]
    test_artifacts.attach_json("commission_frames", frames)
    assert any("INITIALISE data=0xFF" in f for f in frames), \
        "INITIALISE(unaddressed) not observed; saw tail: %s" % frames[-8:]

    assert api.addrs() == before, (before, api.addrs())
    for short in before:
        resp = {}
        for attempt in range(REQUERY_ATTEMPTS):
            if attempt:
                api.count_retry("post_commission_requery",
                                "SA%d silent to QUERY ACTUAL LEVEL after commissioning"
                                % short)
                time.sleep(REQUERY_PAUSE_S)
            resp = api.cmd(short, QUERY_ACTUAL_LEVEL)
            if resp.get("success"):
                break
        assert resp.get("success") is True, (short, resp)