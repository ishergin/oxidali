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


def _series(api, lamp, gap):
    for kelvin in TC_SERIES_KELVIN:
        api.vlamps.ts(lamp, {"color_mode": "cct", "color_temperature_kelvin": kelvin})
        time.sleep(gap)


def _settled(window, dut):
    sent = tripwire.settled_frames(dut, LOG_QUIET_S, LOG_SETTLE_MAX_S, LOG_POLL_S)
    return sent, window.heard_settled(LOG_QUIET_S, LOG_SETTLE_MAX_S, LOG_POLL_S)


def _cannot_vouch(what, missed, lost):
    return ("INCONCLUSIVE, not a product failure: %s the emulator did not hear %d frame(s) "
            "the DUT logged as sent (%s; its loss counters moved by %r), so its frame list "
            "cannot vouch" % (what, len(missed), missed, lost))


@pytest.mark.hil_id("HIL-VG-05")
def test_every_colour_only_write_to_an_emulated_tc_gear_is_activated_without_a_status_gate(
        api, virtual_bench, serial_log, attr_guard, test_artifacts):
    tc_gear = virtual_bench.of_kind("cct")
    if not tc_gear:
        pytest.skip("the park holds no DT8 Tc gear (its shape is %s)" % (virtual_bench.shape,))
    short, oracle = tc_gear[0], virtual_bench.oracle
    last = kelvin_to_mirek(TC_SERIES_KELVIN[-1])
    attr_guard(short, "fade_time_ms", verify=True)
    written = api.wait_op(api.write_attrs(short, {"fade_time_ms": TC_FADE_MS}))
    assert written.get("status") == "succeeded", written
    api.held_tc_mirek(short)
    report = {}
    for gap in TC_SERIES_GAPS_S:
        with oracle.hearing() as window, serial_log.window() as dut:
            _series(api, virtual_bench.vl(short), gap)
            oracle.expect(window, short, "colour_mirek", last)
            sent, heard = _settled(window, dut)
        writes = gearsim.colour_writes(heard, short)
        report[gap] = {"stagings": writes.stagings, "activations": writes.activations,
                       "faults": writes.faults, "unheard": gearsim.unheard(sent, heard),
                       "lost": window.losses(), "held_mirek": api.held_tc_mirek(short)}
    test_artifacts.attach_json("colour_series", report)

    for gap, seen in sorted(report.items()):
        assert not seen["unheard"], _cannot_vouch(
            "during the %.1f s series" % gap, seen["unheard"], seen["lost"])
        assert not seen["faults"], "%.1f s apart: %s" % (gap, seen["faults"])
        assert seen["activations"] >= len(TC_SERIES_KELVIN), (
            "%.1f s apart: %d colour-only writes, %d ACTIVATE frames reached the gear"
            % (gap, len(TC_SERIES_KELVIN), seen["activations"]))
        assert seen["held_mirek"] == last, (
            "%.1f s apart: the gear holds %r mirek after a fresh read, the last command was "
            "%d" % (gap, seen["held_mirek"], last))


STOP_FADE_LEVEL = 150
STOP_FADE_RULE = "hil-vg-06-stop-fade"
STOP_FADE = "stop_fade()"
HEARD_TIMEOUT_S = 10.0
FOLLOW_UP_S = 2.0
LEVEL_WAIT_S = 10.0
POLL_S = 0.3


@pytest.mark.hil_id("HIL-VG-06")
def test_a_stop_fade_rule_sends_one_dapc_mask_to_its_group_and_no_level(
        api, virtual_bench, serial_log, session_rows_guard, rules_guard, op_check,
        test_artifacts):
    if not virtual_bench.groups:
        pytest.skip("no free group: a live lamp's group membership is unknown")
    group, members = virtual_bench.groups[0], virtual_bench.park[:2]
    oracle, mask = virtual_bench.oracle, gearsim.group_dapc_address(group)
    api.groups.join([virtual_bench.vl(s) for s in members], group, timeout_s=OP_TIMEOUT_S)
    api.groups.ts(group, {"power": "on", "level": STOP_FADE_LEVEL})
    assert wait_until(lambda: set(api.actual_levels(members).values()) == {STOP_FADE_LEVEL},
                      LEVEL_WAIT_S, interval_s=POLL_S), api.actual_levels(members)
    rules_guard(http_rule(STOP_FADE_RULE, GROUP_TARGET, group, STOP_FADE))
    with oracle.hearing() as window, serial_log.window() as dut:
        op_check(api.wait_op(api.rules_run(STOP_FADE_RULE)))
        wait_until(lambda: any((f.address, f.data) == (mask, gearsim.MASK)
                               for f in window.heard()), HEARD_TIMEOUT_S, interval_s=POLL_S)
        time.sleep(FOLLOW_UP_S)
        sent, heard = _settled(window, dut)
    missed, lost = gearsim.unheard(sent, heard), window.losses()
    faults = gearsim.stop_fade_faults(heard, group, members)
    levels = api.actual_levels(members)
    test_artifacts.attach_json("stop_fade", {
        "group": group, "members": members, "levels": levels, "faults": faults,
        "unheard": missed, "lost": lost, "sent": sent,
        "heard": [[f.at_us, f.address, f.data] for f in heard]})

    assert not missed, _cannot_vouch("after the rule ran", missed, lost)
    assert not faults, faults
    assert set(levels.values()) == {STOP_FADE_LEVEL}, (
        "the members left the level the stop caught them at: %r" % levels)
    oracle.untouched(window, members)
