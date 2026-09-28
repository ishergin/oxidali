import os

import pytest

from hil.wait import wait_until

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
