import time

import pytest

from hil.wait import wait_until


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
        api.groups.patch(gid, {"name": before["name"] or "Group 15",
                               "ha_entity_enabled": before["ha_entity_enabled"]})


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

    bitmask = None

    def _bit_reported():
        nonlocal bitmask
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
    converge_applies = 0
    res = {}
    for _ in range(3):
        res = api.groups.apply()
        if "operation_id" not in res:
            break
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
