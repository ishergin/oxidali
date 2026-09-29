import threading
import time

import pytest

from hil.api import kelvin_to_mirek
from hil.camera.calibrate import rgb_setpoint, RGB_PRIMARIES
from test_virtual_gear import TC_SERIES_GAPS_S, TC_SERIES_KELVIN


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


TC_FIXTURE_FADE_MS = 1000
TC_FIXTURE_LEVEL = 120
TC_SETTLE_S = 2.5


def _allowed_tc_fixture(api, capabilities):
    present = set(api.present_addrs())
    return next((short for short in api.lamp_addrs()
                 if short in present and capabilities.ensure(short, "cct")), None)


@pytest.mark.hil_id("HIL-TS-09")
def test_a_colour_temperature_series_during_a_fade_leaves_the_fixture_at_the_last_value(
        api, hil_config, capabilities, attr_guard, state_snapshot, test_artifacts):
    if hil_config.lamps_read_only:
        pytest.skip("HIL_LAMPS_READ_ONLY=1: every step here changes a fixture's colour "
                    "temperature, which needs the owner's go-ahead for the run")
    short = _allowed_tc_fixture(api, capabilities)
    if short is None:
        pytest.skip("no DT8 Tc fixture in HIL_LAMP_SHORTS=%s is on the wire: the physics "
                    "half needs one, and the owner's go-ahead" % hil_config.lamp_shorts)
    attr_guard(short, "fade_time_ms", verify=True)
    written = api.wait_op(api.write_attrs(short, {"fade_time_ms": TC_FIXTURE_FADE_MS}))
    assert written.get("status") == "succeeded", written
    api.ts(short, {"power": "on", "level": TC_FIXTURE_LEVEL})
    held = {}
    for gap in TC_SERIES_GAPS_S:
        for kelvin in TC_SERIES_KELVIN:
            api.ts(short, {"color_mode": "cct", "color_temperature_kelvin": kelvin})
            time.sleep(gap)
        time.sleep(TC_SETTLE_S)
        held[gap] = api.held_tc_mirek(short)
    test_artifacts.attach_json("colour_series", {"short": short, "held_mirek": held})
    last = kelvin_to_mirek(TC_SERIES_KELVIN[-1])
    assert all(value == last for value in held.values()), (
        "SA%d does not hold the last colour temperature (%d mirek) after a fresh read: "
        "%r by the gap between commands" % (short, last, held))
    api.off(short)
