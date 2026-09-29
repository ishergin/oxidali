import os
import time

import pytest

from hil import gearsim, tripwire
from hil.api import kelvin_to_mirek
from hil.lamp_guard import GROUP_TARGET, http_rule
from hil.wait import wait_until
from hil_virtual import POLL_S as LOG_POLL_S
from hil_virtual import QUIET_S as LOG_QUIET_S
from hil_virtual import SETTLE_MAX_S as LOG_SETTLE_MAX_S

pytestmark = pytest.mark.virtual_gear

LEVEL = 180
GROUP_LEVEL = 120
SCENE_ID = 15
SCENE_LEVEL = 90
OP_TIMEOUT_S = 60
CONVERGE_S = 5.0


@pytest.mark.hil_id("HIL-VG-01")
def test_a_lamp_level_reaches_its_emulated_gear_and_no_other(api, virtual_bench):
    short, others = virtual_bench.park[0], virtual_bench.park[1:]
    oracle = virtual_bench.oracle
    with oracle.window() as window:
        api.vlamps.ts(virtual_bench.vl(short), {"power": "on", "level": LEVEL})
        oracle.expect(window, short, "level", LEVEL)
        oracle.untouched(window, others)


@pytest.mark.hil_id("HIL-VG-02")
def test_a_group_reaches_its_members_and_no_other_gear(api, virtual_bench):
    if not virtual_bench.groups:
        pytest.skip("no free group: a live lamp's group membership is unknown")
    group = virtual_bench.groups[0]
    members, others = virtual_bench.park[:2], virtual_bench.park[2:]
    oracle = virtual_bench.oracle
    with oracle.window() as window:
        api.groups.join([virtual_bench.vl(s) for s in members], group, timeout_s=OP_TIMEOUT_S)
        for short in members:
            oracle.expect(window, short, "groups", "0x%04x" % (1 << group))
    with oracle.window() as window:
        api.groups.ts(group, {"power": "on", "level": GROUP_LEVEL})
        for short in members:
            oracle.expect(window, short, "level", GROUP_LEVEL)
        oracle.untouched(window, others)


@pytest.mark.hil_id("HIL-VG-03")
def test_a_scene_is_stored_in_the_emulated_gear(api, virtual_bench):
    short = virtual_bench.park[0]
    lamp = virtual_bench.vl(short)
    oracle = virtual_bench.oracle
    rows = [{"virtual_lamp_id": lamp,
             "desired": {"included": True, "power": "on", "level": SCENE_LEVEL}}]
    with oracle.window() as window:
        api.scenes.matrix_patch(SCENE_ID, rows)
        view = api.wait_op(api.scenes.apply(SCENE_ID), timeout_s=OP_TIMEOUT_S)
        assert view.get("status") == "succeeded", view
        oracle.expect(window, short, "scene[%d]" % SCENE_ID, SCENE_LEVEL)

    def _converged():
        row = {r["virtual_lamp_id"]: r for r in api.scenes.matrix(SCENE_ID)["rows"]}[lamp]
        return not row["dirty"]

    wait_until(_converged, CONVERGE_S, interval_s=0.3)


SEARCH_TOP = 0xFFFFFF
QUERY_STATUS = 0x90
COMMISSIONING_ENV = "HIL_ALLOW_VIRTUAL_COMMISSIONING"


def _step(api, step, body=None):
    reply = api._req("POST", "adapters/%d/commissioning/steps/%s" % (api.adapter, step),
                     body or {})
    assert reply.get("success") is not False or step in ("compare", "verify-short-address"), \
        "%s: %s" % (step, reply)
    return reply


def _any_unaddressed(api):
    _step(api, "initialise", {"scope": "unaddressed"})
    try:
        _step(api, "search-address", {"search_address": SEARCH_TOP})
        return bool(_step(api, "compare").get("match"))
    finally:
        _step(api, "terminate")


def _lowest_random(api):
    lo, hi = 0, SEARCH_TOP
    while lo < hi:
        mid = (lo + hi) // 2
        _step(api, "search-address", {"search_address": mid})
        if _step(api, "compare").get("match"):
            hi = mid
        else:
            lo = mid + 1
    return lo


@pytest.mark.hil_id("HIL-VG-04")
@pytest.mark.destructive
def test_an_unaddressed_emulated_gear_is_commissioned_onto_the_park(api, virtual_bench):
    if os.environ.get(COMMISSIONING_ENV) != "1":
        pytest.skip("commissioning on the installation's wire needs %s=1 and a go-ahead "
                    "for this run" % COMMISSIONING_ENV)
    sim, oracle, target = virtual_bench.sim, virtual_bench.oracle, virtual_bench.park[0]
    sim.disable("all")
    assert not _any_unaddressed(api), "a live gear on this wire has no short address"
    sim.command("unaddress 1")
    sim.enable("all")
    assert _any_unaddressed(api), "the positive control stayed silent"
    with oracle.window() as window:
        _step(api, "initialise", {"scope": "unaddressed"})
        try:
            _step(api, "randomise")
            _step(api, "search-address", {"search_address": _lowest_random(api)})
            _step(api, "program-short-address", {"short_address": target})
            assert _step(api, "verify-short-address", {"short_address": target}).get("match")
            _step(api, "withdraw")
        finally:
            _step(api, "terminate")
        oracle.expect(window, target, "short_address", target)
    reply = api.cmd(target, QUERY_STATUS)
    assert reply.get("success") and not reply.get("backward_violation"), \
        "SA%d does not answer at its new address: %r" % (target, reply)


TC_SERIES_KELVIN = (2700, 3500, 4200, 5000, 6000, 3000)
TC_SERIES_GAPS_S = (0.3, 3.0)
TC_FADE_MS = 1000
COLOUR_FIELD = "colour_mirek"


def _series(api, lamp, gap):
    for kelvin in TC_SERIES_KELVIN:
        api.vlamps.ts(lamp, {"color_mode": "cct", "color_temperature_kelvin": kelvin})
        time.sleep(gap)


def _settled(window, dut):
    sent = tripwire.settled_frames(dut, LOG_QUIET_S, LOG_SETTLE_MAX_S, LOG_POLL_S)
    return sent, window.heard_settled(LOG_QUIET_S, LOG_SETTLE_MAX_S, LOG_POLL_S)


def _reached(oracle, window, short, value):
    return any(c.short == short and c.field == COLOUR_FIELD and c.new == str(value)
               for c in oracle.changes(window))


def _colour_series(api, bench, serial_log, short, gap):
    last = kelvin_to_mirek(TC_SERIES_KELVIN[-1])
    losses = tripwire.log_losses(api.stats())
    with bench.oracle.hearing() as window, serial_log.window() as dut:
        _series(api, bench.vl(short), gap)
        wait_until(lambda: _reached(bench.oracle, window, short, last), HEARD_TIMEOUT_S,
                   interval_s=POLL_S)
        sent, heard = _settled(window, dut)
    writes = gearsim.colour_writes(heard, short)
    return {"gates": writes.gates, "faults": writes.faults, "activated": writes.activated(),
            "activates_sent": sent.count((gearsim.command_address(short), gearsim.ACTIVATE)),
            "unheard": gearsim.unheard(sent, heard), "emulator_lost": window.losses(),
            "dut_lost": tripwire.lost_lines(losses, tripwire.log_losses(api.stats())),
            "held_mirek": api.held_tc_mirek(short)}


def _cannot_vouch(what, seen):
    return ("INCONCLUSIVE, not a product failure: %s the controller's log lost lines (%r) or "
            "the emulator did not hear %d frame(s) the controller logged as sent (%s; its own "
            "loss counters moved by %r), so the frame lists cannot vouch"
            % (what, seen["dut_lost"], len(seen["unheard"]), seen["unheard"],
               seen["emulator_lost"]))


@pytest.mark.hil_id("HIL-VG-05")
def test_every_colour_only_write_to_an_emulated_tc_gear_is_activated_without_a_status_gate(
        api, virtual_bench, serial_log, attr_guard, test_artifacts):
    tc_gear = virtual_bench.of_kind("cct")
    if not tc_gear:
        pytest.skip("the park holds no DT8 Tc gear (its shape is %s)" % (virtual_bench.shape,))
    short = tc_gear[0]
    commanded = [kelvin_to_mirek(k) for k in TC_SERIES_KELVIN]
    attr_guard(short, "fade_time_ms", verify=True, required=True)
    written = api.wait_op(api.write_attrs(short, {"fade_time_ms": TC_FADE_MS}))
    assert written.get("status") == "succeeded", written
    api.held_tc_mirek(short)
    report = {gap: _colour_series(api, virtual_bench, serial_log, short, gap)
              for gap in TC_SERIES_GAPS_S}
    test_artifacts.attach_json("colour_series", report)

    for gap, seen in sorted(report.items()):
        assert not seen["gates"], "%.1f s apart: %s" % (gap, seen["gates"])
    for gap, seen in sorted(report.items()):
        assert not seen["dut_lost"] and not seen["unheard"], _cannot_vouch(
            "during the %.1f s series" % gap, seen)
        assert seen["activates_sent"] >= len(commanded), (
            "%.1f s apart: the controller logged %d ACTIVATE frames to SA%d for %d colour-only "
            "writes, with no log line lost" % (gap, seen["activates_sent"], short,
                                               len(commanded)))
        assert not seen["faults"], "%.1f s apart: %s" % (gap, seen["faults"])
        assert seen["activated"] == commanded, (
            "%.1f s apart: the gear activated %r mirek, the commands were %r"
            % (gap, seen["activated"], commanded))
        assert seen["held_mirek"] == commanded[-1], (
            "%.1f s apart: the gear holds %r mirek after a fresh read, the last command was "
            "%d" % (gap, seen["held_mirek"], commanded[-1]))


STOP_FADE_LEVEL = 150
STOP_FADE_RULE = "hil-vg-06-stop-fade"
STOP_FADE = "stop_fade()"
HEARD_TIMEOUT_S = 10.0
LEVEL_WAIT_S = 10.0
POLL_S = 0.3


def _stop_report(api, window, group, dut_lines, losses):
    sent, heard = _settled(window, dut_lines)
    mask = (gearsim.group_dapc_address(group), gearsim.MASK)
    return {"sent_masks": sent.count(mask),
            "collided_masks": tripwire.collided_frames(dut_lines.lines()).count(mask),
            "heard_masks": gearsim.mask_frames(heard, group), "heard": heard,
            "unheard": gearsim.unheard(sent, heard), "emulator_lost": window.losses(),
            "dut_lost": tripwire.lost_lines(losses, tripwire.log_losses(api.stats()))}


@pytest.mark.hil_id("HIL-VG-06")
def test_a_stop_fade_rule_sends_one_dapc_mask_to_its_group_and_no_level(
        api, virtual_bench, serial_log, rules_guard, session_rows_guard, op_check,
        test_artifacts):
    if not virtual_bench.groups:
        pytest.skip("no free group: a live lamp's group membership is unknown")
    group, members = virtual_bench.groups[0], virtual_bench.park[:2]
    oracle = virtual_bench.oracle
    api.groups.join([virtual_bench.vl(s) for s in members], group, timeout_s=OP_TIMEOUT_S)
    api.groups.ts(group, {"power": "on", "level": STOP_FADE_LEVEL})
    assert wait_until(lambda: set(api.actual_levels(members).values()) == {STOP_FADE_LEVEL},
                      LEVEL_WAIT_S, interval_s=POLL_S), api.actual_levels(members)
    rules_guard(http_rule(STOP_FADE_RULE, GROUP_TARGET, group, STOP_FADE))
    losses = tripwire.log_losses(api.stats())
    with oracle.hearing() as window, serial_log.window() as dut:
        op_check(api.wait_op(api.rules_run(STOP_FADE_RULE)))
        wait_until(lambda: gearsim.mask_frames(window.heard(), group), HEARD_TIMEOUT_S,
                   interval_s=POLL_S)
        seen = _stop_report(api, window, group, dut, losses)
    moves = gearsim.level_moves(seen.pop("heard"), group, members)
    levels = api.actual_levels(members)
    test_artifacts.attach_json("stop_fade", dict(seen, group=group, members=members,
                                                 moves=moves, levels=levels))

    assert not moves, moves
    assert seen["sent_masks"] <= 1, (
        "the controller logged %d complete DAPC MASK frames to group %d for one stop"
        % (seen["sent_masks"], group))
    assert seen["heard_masks"] <= seen["sent_masks"] + seen["collided_masks"], (
        "the gear heard %d DAPC MASK frames to group %d, the controller sent %d and %d "
        "collided" % (seen["heard_masks"], group, seen["sent_masks"], seen["collided_masks"]))
    assert not seen["dut_lost"] and not seen["unheard"], _cannot_vouch(
        "after the rule ran", seen)
    assert seen["sent_masks"] == 1 and seen["heard_masks"] >= 1, (
        "no DAPC MASK to group %d reached the gear: the controller logged %d, the gear heard "
        "%d, and neither log lost a line" % (group, seen["sent_masks"], seen["heard_masks"]))
    assert set(levels.values()) == {STOP_FADE_LEVEL}, (
        "the members left the level the stop caught them at: %r" % levels)
    oracle.untouched(window, members)
