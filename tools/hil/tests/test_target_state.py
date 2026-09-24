import threading

import pytest

from hil.camera.calibrate import rgb_setpoint, RGB_PRIMARIES


@pytest.mark.hil_id("HIL-TS-05")
@pytest.mark.sniffer
def test_burst_coalescing_supersede(api, lamps, sniffer, wait_state,
                                    state_snapshot, test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    levels = (60, 120, 180, 240)
    results = {}

    def put(level):
        results[level] = api.raw_request(
            "PUT", "adapters/%d/physical-devices/%d/target-state"
            % (api.adapter, short), {"power": "on", "level": level})

    with sniffer.window() as win:
        threads = [threading.Thread(target=put, args=(lvl,)) for lvl in levels]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        frames = [f["decoded"] for f in win.frames()]
    test_artifacts.attach_json(
        "burst", {"responses": {str(k): v for k, v in results.items()},
                  "frames": frames})
    statuses = {lvl: st for lvl, (st, _) in results.items()}
    assert set(statuses.values()) <= {200, 409}, statuses
    for lvl, (st, body) in results.items():
        if st == 409:
            assert body.get("error") == "superseded", body
    confirmed = [lvl for lvl, st in statuses.items() if st == 200]
    assert confirmed, "at least one burst PUT must confirm"
    last = wait_state(short, lambda s: s.get("level") in confirmed)
    assert last.get("level") in confirmed, (last, confirmed)
    api.off(short)


@pytest.mark.hil_id("HIL-TS-06")
def test_off_all_converges_states(api, lamps, wait_state, state_snapshot):
    import time
    for label in lamps.labels():
        api.ts(lamps.by_label[label], {"power": "on", "level": 150})
        time.sleep(0.35)
    api.off_all()
    for label in lamps.labels():
        short = lamps.by_label[label]
        last = wait_state(short, lambda s: s.get("power") == "off")
        assert last.get("power") == "off", (short, last)


@pytest.mark.hil_id("HIL-TS-07")
@pytest.mark.sniffer
@pytest.mark.needs_capability("rgb")
def test_combined_level_and_rgb_single_put(api, capabilities,
                                           needs_capability, sniffer, paced,
                                           wait_state, state_snapshot,
                                           test_artifacts):
    short = capabilities.any_lamp_with("rgb")
    _, red = RGB_PRIMARIES[0]
    setpoint = rgb_setpoint(red)
    with sniffer.window() as win:
        paced(1.0)
        api.ts(short, setpoint)
        win.expect_frame("DAPC short %d -> level 200" % short)
        win.expect_frame("DT8 SET TEMP RGB DIM LEVEL")
    last = wait_state(short, lambda s: s.get("level") == 200)
    test_artifacts.attach_json("state", last)
    assert last.get("level") == 200, last
    if last.get("rgb"):
        assert last["rgb"].get("r", 0) >= last["rgb"].get("b", 0), last
    api.off(short)
