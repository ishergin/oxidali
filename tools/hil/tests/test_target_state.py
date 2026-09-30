import threading
import time

import pytest

from hil.api import kelvin_to_mirek
from hil.camera.calibrate import rgb_setpoint, RGB_PRIMARIES
from hil.wait import wait_until
from test_virtual_gear import TC_SERIES_GAPS_S, TC_SERIES_KELVIN

pytestmark = pytest.mark.light


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
CLAIM_GAP_S, CONTROL_GAP_S = TC_SERIES_GAPS_S
QUERY_STATUS = 0x90
FADE_RUNNING = 0x10
SETTLE_WAIT_S = 15.0
SETTLE_POLL_S = 0.3
MS_PER_S = 1000


@pytest.fixture()
def tc_fixture(api, hil_config, capabilities):
    present = set(api.present_addrs())
    short = next((s for s in api.lamp_addrs()
                  if s in present and capabilities.ensure(s, "cct")), None)
    if short is None:
        pytest.skip("no DT8 Tc fixture in HIL_LAMP_SHORTS=%s is on the wire: the physics "
                    "half needs one, and the owner's go-ahead" % hil_config.lamp_shorts)
    return short


def _fade_running(api, short):
    reply = api.cmd(short, QUERY_STATUS)
    if not reply.get("success") or reply.get("backward_violation"):
        return None
    return bool(reply.get("backward_frame", 0) & FADE_RUNNING)


def _settled(api, short, level=None):
    def still():
        if level is not None and api.actual_level(short) != level:
            return False
        return _fade_running(api, short) is False
    return bool(wait_until(still, SETTLE_WAIT_S, interval_s=SETTLE_POLL_S))


def _timed_series(api, short, gap):
    ends, running, retried = [], None, sum(api.retries.values())
    for index, kelvin in enumerate(TC_SERIES_KELVIN):
        if index == len(TC_SERIES_KELVIN) - 1:
            running = _fade_running(api, short)
        api.ts(short, {"color_mode": "cct", "color_temperature_kelvin": kelvin})
        ends.append(time.monotonic())
        time.sleep(gap)
    settled = _settled(api, short)
    return {"widest_gap_s": max(b - a for a, b in zip(ends, ends[1:])),
            "fade_running_before_last": running, "settled": settled,
            "retried": sum(api.retries.values()) - retried,
            "held_mirek": api.held_tc_mirek(short)}


@pytest.mark.hil_id("HIL-TS-09")
def test_a_colour_temperature_series_during_a_fade_leaves_the_fixture_at_the_last_value(
        api, tc_fixture, attr_guard, state_snapshot, test_artifacts):
    short, last = tc_fixture, kelvin_to_mirek(TC_SERIES_KELVIN[-1])
    attr_guard(short, "fade_time_ms", verify=True, required=True)
    written = api.wait_op(api.write_attrs(short, {"fade_time_ms": TC_FIXTURE_FADE_MS}))
    assert written.get("status") == "succeeded", written
    api.ts(short, {"power": "on", "level": TC_FIXTURE_LEVEL, "color_mode": "cct",
                   "color_temperature_kelvin": TC_SERIES_KELVIN[0]})
    ready = _settled(api, short, TC_FIXTURE_LEVEL)
    before = api.held_tc_mirek(short)
    claim = _timed_series(api, short, CLAIM_GAP_S) if ready and before != last else {}
    control = _timed_series(api, short, CONTROL_GAP_S) if claim else {}
    test_artifacts.attach_json("colour_series", {"short": short, "before_mirek": before,
                                                 "claim": claim, "control": control})
    api.off(short)

    assert ready and before != last, (
        "INCONCLUSIVE, not a product failure: the fixture %s and holds %r mirek before the "
        "series, which must start away from the last value %d"
        % ("settled" if ready else "never settled at level %d" % TC_FIXTURE_LEVEL,
           before, last))
    assert claim["widest_gap_s"] < TC_FIXTURE_FADE_MS / MS_PER_S and not claim["retried"], (
        "INCONCLUSIVE, not a product failure: commands %.1f s apart ended %.2f s apart at "
        "worst, with %d retried request(s), so a %d ms fade had ended before the next "
        "arrived" % (CLAIM_GAP_S, claim["widest_gap_s"], claim["retried"],
                     TC_FIXTURE_FADE_MS))
    assert claim["fade_running_before_last"] is True, (
        "INCONCLUSIVE, not a product failure: QUERY STATUS before the last command reported "
        "fade running %r, so a gate on it would not have skipped anything"
        % claim["fade_running_before_last"])
    assert control["settled"] and control["held_mirek"] == last, (
        "INCONCLUSIVE, not a product failure: the control series %.0f s apart left the "
        "fixture at %r mirek, not %d, so the fixture does not hold a slow series either"
        % (CONTROL_GAP_S, control.get("held_mirek"), last))
    assert claim["held_mirek"] == last, (
        "SA%d holds %r mirek after the series %.1f s apart during a fade, the last command "
        "was %d" % (short, claim["held_mirek"], CLAIM_GAP_S, last))
