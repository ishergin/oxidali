import time

import pytest

from hil.lamp_guard import GROUP_TARGET, TARGET_SEGMENT, http_rule
from hil.wait import wait_until
from hil_test_guards import ask_guard, drive_allowed


def _membership(device):
    if "groups_membership" in device:
        return device.get("groups_membership")
    return (((device.get("attributes") or {}).get("groups") or {})
            .get("membership") or {}).get("value")


@pytest.mark.hil_id("HIL-GRP-03")
def test_group_patch_metadata_roundtrip(api):
    gid = 15
    before = api.groups.get(gid)
    try:
        dto = api.groups.patch(gid, {"name": "hil-grp-tmp",
                                     "ha_entity_enabled": True})
        assert dto["name"] == "hil-grp-tmp" and dto["ha_entity_enabled"] is True
        listed = next(g for g in api.groups.list()["groups"]
                      if g["group_id"] == gid)
        assert listed["name"] == "hil-grp-tmp"
    finally:
        api.groups.patch(gid, {"ha_entity_enabled": before["ha_entity_enabled"]})
        api.groups.patch(gid, {"name": before["name"] or None})
    assert api.groups.get(gid)["name"] == before["name"]


@pytest.mark.hil_id("HIL-GRP-04")
def test_matrix_counts_and_dirty_consistent(api, vl_bindings, test_artifacts):
    matrix = api.groups.matrix()
    groups = api.groups.list()["groups"]
    test_artifacts.attach_json("matrix_groups", matrix["groups"])
    assert len(matrix["rows"]) == 64
    matrix_groups = {g["group_id"]: g for g in matrix["groups"]}
    any_dirty = False
    for g in groups:
        gid = g["group_id"]
        desired = sum(1 for r in matrix["rows"] if r["desired"][gid])
        applied = sum(1 for r in matrix["rows"] if r["applied"][gid])
        row_dirty = any(r["desired"][gid] != r["applied"][gid]
                        for r in matrix["rows"])
        any_dirty = any_dirty or row_dirty
        assert g["member_count_desired"] == desired, (gid, g)
        assert g["member_count_applied"] == applied, (gid, g)
        assert g["dirty"] == row_dirty == matrix_groups[gid]["dirty"], (gid, g)
    assert matrix["dirty"] == any_dirty


@pytest.mark.hil_id("HIL-GRP-05")
@pytest.mark.sniffer
def test_matrix_patch_marks_dirty_without_frames(api, vl_bindings, lamps,
                                                 free_group,
                                                 group_matrix_guard, sniffer,
                                                 ops_quiesce):
    label = lamps.labels()[0]
    lid = label - 1
    row = next(r for r in group_matrix_guard["rows"]
               if r["virtual_lamp_id"] == lid)
    desired = list(row["desired"])
    desired[free_group] = True
    with sniffer.window() as win:
        api.groups.matrix_patch([{"virtual_lamp_id": lid, "desired": desired}])
        matrix = api.groups.matrix()
        g = next(g for g in matrix["groups"] if g["group_id"] == free_group)
        assert g["dirty"] is True
        grp = api.groups.get(free_group)
        assert grp["member_count_desired"] == 1
        assert grp["member_count_applied"] == 0
        win.expect_quiet("ADD TO GROUP", settle_s=2.5)


@pytest.mark.light
@pytest.mark.hil_id("HIL-GRP-02")
@pytest.mark.sniffer
def test_group_apply_programs_gear(api, vl_bindings, lamps, free_group,
                                   group_matrix_guard, sniffer, paced,
                                   ops_quiesce, capabilities, wait_state,
                                   state_snapshot, test_artifacts, op_check):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    api.groups.join([lid], free_group, apply=False)

    with sniffer.window() as win:
        paced(1.0)
        op = api.groups.apply()
        assert op.get("type") == "group_apply", op
        view = api.wait_op(op, timeout_s=60)
        test_artifacts.attach_json("apply_op", view)
        view = op_check(view)
        result = view.get("result") or {}
        assert not result.get("failed"), result
        programmed = result.get("programmed") or []
        assert any(p["virtual_lamp_id"] == lid and p["group_id"] == free_group
                   and p["action"] == "add" for p in programmed), result
        if "failed_total" in result:
            assert result["failed_total"] == 0, result
        win.expect_program_witness(
            "ADD TO GROUP %d (short %d)" % (free_group, short),
            "QUERY GROUPS 0-7 (short %d)" % short)

    row_after = {}

    def _applied():
        nonlocal row_after
        row_after = next(r for r in api.groups.matrix()["rows"]
                         if r["virtual_lamp_id"] == lid)
        return row_after["applied"][free_group]

    wait_until(_applied, 3.0, interval_s=0.3)
    assert row_after["applied"][free_group] is True, row_after
    assert row_after["desired"][free_group] is True, row_after

    bitmask, reads = None, 0

    def _bit_reported():
        nonlocal bitmask, reads
        if reads:
            api.count_retry("group_membership_reread",
                            "SA%d: group %d missing from the gear's mask 0x%04x"
                            % (short, free_group, bitmask or 0))
        reads += 1
        api.attr_read_checked(short, groups="groups")
        bitmask = _membership(api.attributes(short, ["groups"]))
        return (bitmask or 0) >> free_group & 1

    wait_until(_bit_reported, 6.0, interval_s=1.0)
    assert bitmask is not None and (bitmask >> free_group) & 1 == 1, bitmask

    group_setpoint = {"power": "on", "level": 200}
    with sniffer.window() as win:
        paced(1.0)
        resp = api.group_ts(free_group, group_setpoint)
        assert resp.get("status") == "accepted", resp
        win.expect_frame("DAPC group %d -> level 200" % free_group,
                         resend=lambda: api.group_ts(free_group,
                                                     group_setpoint))
    time.sleep(1.0)
    test_artifacts.attach_json(
        "member_state_after_group_dapc",
        wait_state(short, lambda s: s.get("level") == 200, timeout_s=2.0))
    api.off(short)


@pytest.mark.hil_id("HIL-GRP-06")
def test_apply_empty_diff_returns_matrix(api, vl_bindings, ops_quiesce,
                                         test_artifacts):
    ask_guard(api, "POST", "adapters/%d/groups/apply" % api.adapter)
    converge_applies = 0
    res = {}
    for _ in range(3):
        res = api.groups.apply()
        if "operation_id" not in res:
            break
        if converge_applies:
            api.count_retry("group_apply_reconverge",
                            "apply %d still carried a diff" % (converge_applies + 1))
        converge_applies += 1
        api.wait_op(res, timeout_s=60)
    test_artifacts.attach_json("empty_apply",
                               {"converge_applies": converge_applies,
                                **{k: res[k] for k in res if k != "rows"}})
    assert "operation_id" not in res and "rows" in res, res
    before_keys = set(api.operations())
    res = api.groups.apply()
    assert "operation_id" not in res and "rows" in res, res
    assert not {k for k in set(api.operations()) - before_keys
                if k.startswith("grp-apply")}


@pytest.mark.hil_id("HIL-GRP-07")
@pytest.mark.sniffer
def test_concurrent_apply_conflicts(api, vl_bindings, lamps, free_group,
                                    group_matrix_guard, ops_quiesce,
                                    state_snapshot, test_artifacts, op_check):
    api.groups.join([label - 1 for label in lamps.labels()[:3]], free_group,
                    apply=False)

    first = api.groups.apply()
    assert "operation_id" in first, first
    status, body = api.raw_request(
        "POST", "adapters/%d/groups/apply" % api.adapter)
    test_artifacts.attach_json("second_apply", {"status": status, "body": body})
    view = api.wait_op(first, timeout_s=60)
    view = op_check(view)
    if status != 409:
        pytest.skip("first apply finished before the second POST landed "
                    "(no overlap window on this run) — observed %d" % status)
    assert body.get("error") == "conflict", body


STOP_FADE_RULE = "hil-grp-10-stop-fade"
STOP_FADE = "stop_fade()"
STOP_FADE_FADE_MS = 4000
STOP_FADE_LOW, STOP_FADE_HIGH = 40, 240
STOP_FADE_CATCH_S = 1.2
STOP_FADE_SETTLE_S = 6.0
STOP_FADE_LEVEL_WAIT_S = 12.0
STOP_FADE_POLL_S = 0.5
STOP_FADE_STEADY_S = STOP_FADE_FADE_MS / 1000 + 2.0


def _steady_levels(api, members):
    seen = [api.actual_levels(members)]

    def steady():
        seen.append(api.actual_levels(members))
        return seen[-1] == seen[-2]

    wait_until(steady, STOP_FADE_STEADY_S, interval_s=STOP_FADE_POLL_S)
    return seen[-1]


def _stop_mid_fade(api, members, rule, op_check, go_high):
    go_high()
    time.sleep(STOP_FADE_CATCH_S)
    op_check(api.wait_op(api.rules_run(rule)))
    first = api.actual_levels(members)
    caught = _steady_levels(api, members)
    went_on = wait_until(lambda: api.actual_levels(members) != caught, STOP_FADE_SETTLE_S,
                         interval_s=STOP_FADE_POLL_S)
    return {"first": first, "caught": caught, "later": api.actual_levels(members),
            "went_on": went_on}


def _assert_stopped(stop, what):
    assert all(level is not None and STOP_FADE_LOW < level < STOP_FADE_HIGH
               for level in stop["first"].values()), (
        "INCONCLUSIVE, not a product failure: the stop did not land inside the %d ms fade "
        "from %d to %d (first read %r)"
        % (STOP_FADE_FADE_MS, STOP_FADE_LOW, STOP_FADE_HIGH, stop["first"]))
    assert all(level is not None and level < STOP_FADE_HIGH
               for level in stop["caught"].values()), (
        "%s faded on to %d after .stop_fade(): %r" % (what, STOP_FADE_HIGH, stop))
    assert not stop["went_on"] and stop["later"] == stop["caught"], (
        "%s went on after .stop_fade(): %r" % (what, stop))


@pytest.fixture()
def group_actions_allowed(api):
    drive_allowed(api, TARGET_SEGMENT, "a stop_fade rule, whose DAPC MASK reaches every "
                                       "member of its group,")


@pytest.mark.light
@pytest.mark.hil_id("HIL-GRP-10")
def test_a_stop_fade_rule_leaves_real_members_where_it_caught_them(
        api, group_actions_allowed, rules_guard, attr_guard, vl_bindings, lamps, free_group,
        group_matrix_guard, ops_quiesce, state_snapshot, op_check, test_artifacts):
    members = [lamps.by_label[label] for label in lamps.labels()]
    for short in members:
        attr_guard(short, "fade_time_ms", verify=True, required=True)
        written = api.wait_op(api.write_attrs(short, {"fade_time_ms": STOP_FADE_FADE_MS}))
        assert written.get("status") == "succeeded", written
    api.groups.join([label - 1 for label in lamps.labels()], free_group)
    rules_guard(http_rule(STOP_FADE_RULE, GROUP_TARGET, free_group, STOP_FADE))
    api.group_ts(free_group, {"power": "on", "level": STOP_FADE_LOW})
    assert wait_until(lambda: set(api.actual_levels(members).values()) == {STOP_FADE_LOW},
                      STOP_FADE_LEVEL_WAIT_S, interval_s=STOP_FADE_POLL_S), \
        api.actual_levels(members)

    stop = _stop_mid_fade(api, members, STOP_FADE_RULE, op_check,
                          lambda: api.group_ts(free_group,
                                               {"power": "on", "level": STOP_FADE_HIGH}))
    test_artifacts.attach_json("stop_fade", dict(stop, group=free_group))
    _assert_stopped(stop, "the members")


LAMP_TARGET = "lamp"
STOP_FADE_LAMP_RULE = "hil-grp-11-stop-fade"


@pytest.mark.light
@pytest.mark.hil_id("HIL-GRP-11")
def test_a_stop_fade_rule_leaves_a_real_lamp_where_it_caught_it(
        api, rules_guard, attr_guard, vl_bindings, lamps, state_snapshot, op_check,
        test_artifacts):
    label = lamps.labels()[0]
    vl, short = label - 1, lamps.by_label[label]
    attr_guard(short, "fade_time_ms", verify=True, required=True)
    written = api.wait_op(api.write_attrs(short, {"fade_time_ms": STOP_FADE_FADE_MS}))
    assert written.get("status") == "succeeded", written
    rules_guard(http_rule(STOP_FADE_LAMP_RULE, LAMP_TARGET, vl, STOP_FADE))
    api.ts(short, {"power": "on", "level": STOP_FADE_LOW})
    assert wait_until(lambda: api.actual_level(short) == STOP_FADE_LOW,
                      STOP_FADE_LEVEL_WAIT_S, interval_s=STOP_FADE_POLL_S), \
        api.actual_level(short)

    stop = _stop_mid_fade(api, [short], STOP_FADE_LAMP_RULE, op_check,
                          lambda: api.ts(short, {"power": "on", "level": STOP_FADE_HIGH}))
    test_artifacts.attach_json("stop_fade", dict(stop, lamp=short))
    _assert_stopped(stop, "the lamp")
