import time

import pytest

from hil.wait import wait_until

SCENE_ID = 12
LEVEL_A, LEVEL_B = 200, 60


def _rows_for(lamps, vl_bindings):
    labels = lamps.labels()
    if len(labels) < 2:
        pytest.skip("scene tests need at least two calibrated lamps")
    a, b = labels[0] - 1, labels[1] - 1
    return [
        {"virtual_lamp_id": a,
         "desired": {"included": True, "power": "on", "level": LEVEL_A}},
        {"virtual_lamp_id": b,
         "desired": {"included": True, "power": "on", "level": LEVEL_B}},
    ]


@pytest.mark.hil_id("HIL-SCN-03")
def test_scene_list_and_patch_metadata(api, scenes_supported):
    scenes = api.scenes.list()["scenes"]
    assert len(scenes) == 16
    before = api.scenes.get(SCENE_ID)
    try:
        dto = api.scenes.patch(SCENE_ID, {"name": "hil-scene-tmp",
                                          "ha_select_enabled": True})
        assert dto["name"] == "hil-scene-tmp"
        assert dto["ha_select_enabled"] is True
        listed = next(s for s in api.scenes.list()["scenes"]
                      if s["scene_id"] == SCENE_ID)
        assert listed["name"] == "hil-scene-tmp"
    finally:
        api.scenes.patch(SCENE_ID, {
            "name": before["name"] or ("Scene %d" % SCENE_ID),
            "ha_select_enabled": before["ha_select_enabled"]})


@pytest.mark.hil_id("HIL-SCN-04")
@pytest.mark.sniffer
def test_scene_matrix_patch_stages_without_frames(api, scenes_supported,
                                                  vl_bindings, lamps,
                                                  scene_matrix_guard, sniffer,
                                                  ops_quiesce):
    scene_matrix_guard(SCENE_ID)
    rows = _rows_for(lamps, vl_bindings)
    with sniffer.window() as win:
        api.scenes.matrix_patch(SCENE_ID, rows)
        dto = api.scenes.get(SCENE_ID)
        assert dto["dirty"] is True
        assert dto["row_count_included"] >= len(rows)
        win.expect_quiet("STORE DTR AS SCENE", settle_s=2.5)


@pytest.mark.hil_id("HIL-SCN-01")
@pytest.mark.sniffer
def test_scene_apply_programs_gear(api, scenes_supported, vl_bindings, lamps,
                                   scene_matrix_guard, sniffer, paced,
                                   ops_quiesce, state_snapshot,
                                   test_artifacts, op_check):
    scene_matrix_guard(SCENE_ID)
    api.cmd_wire(0xFF, 0x50 + SCENE_ID, repeat=2)
    rows = _rows_for(lamps, vl_bindings)
    api.scenes.matrix_patch(SCENE_ID, rows)
    with sniffer.window() as win:
        paced(1.0)
        op = api.scenes.apply(SCENE_ID)
        assert op.get("type") == "scene_apply", op
        view = api.wait_op(op, timeout_s=60)
        test_artifacts.attach_json("scene_apply_op", view)
        view = op_check(view)
        result = view.get("result") or {}
        assert result.get("failed_total") == 0, result
        win.expect_program_witness(
            "STORE DTR AS SCENE %d" % SCENE_ID,
            "QUERY SCENE LEVEL %d" % SCENE_ID)
    wanted = {row["virtual_lamp_id"] for row in rows}
    by_vl = {}

    def _converged():
        nonlocal by_vl
        by_vl = {r["virtual_lamp_id"]: r
                 for r in api.scenes.matrix(SCENE_ID)["rows"]}
        return all(not by_vl[vl]["dirty"] for vl in wanted)

    wait_until(_converged, 3.0, interval_s=0.3)
    for row in rows:
        after = by_vl[row["virtual_lamp_id"]]
        assert after["dirty"] is False, after
        assert after["applied"]["included"] is True
        assert after["applied"]["level"] == row["desired"]["level"]


@pytest.mark.hil_id("HIL-SCN-02")
@pytest.mark.sniffer
@pytest.mark.optical
def test_scene_recall_reaches_gear_and_states(api, scenes_supported,
                                              vl_bindings, lamps,
                                              scene_matrix_guard, sniffer,
                                              paced, ops_quiesce,
                                              camera_oracle, wait_state,
                                              state_snapshot, test_artifacts):
    scene_matrix_guard(SCENE_ID)
    api.cmd_wire(0xFF, 0x50 + SCENE_ID, repeat=2)
    rows = _rows_for(lamps, vl_bindings)
    api.scenes.matrix_patch(SCENE_ID, rows)
    api.wait_op(api.scenes.apply(SCENE_ID), timeout_s=60)

    before_keys = set(api.operations())
    with sniffer.window() as win:
        paced(1.0)
        resp = api.scenes.recall(SCENE_ID)
        assert resp.get("status") == "confirmed", resp
        win.expect_frame("GO TO SCENE %d (broadcast)" % SCENE_ID)
        label_a = rows[0]["virtual_lamp_id"] + 1
        camera_oracle.assert_on(label_a, window=win, name="scene_member")
    assert not {k for k in set(api.operations()) - before_keys}, \
        "recall must not create an operation"
    short_a = lamps.by_label[label_a]
    last = wait_state(short_a, lambda s: s.get("level") == LEVEL_A)
    test_artifacts.attach_json("member_state", last)
    assert last.get("level") == LEVEL_A, last
    assert last.get("last_dapc_source") == "scene", last
    api.off_all()


@pytest.mark.hil_id("HIL-SCN-05")
def test_recall_of_empty_scene_is_noop(api, scenes_supported, vl_bindings,
                                       lamps, wait_state, state_snapshot,
                                       test_artifacts):
    empty_sid = next(s["scene_id"] for s in api.scenes.list()["scenes"]
                     if s["row_count_included"] == 0)
    resp = api.scenes.apply(empty_sid)
    if "operation_id" in resp:
        api.wait_op(resp, timeout_s=60)
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    api.ts(short, {"power": "on", "level": 111})
    assert wait_state(short, lambda s: s.get("level") == 111).get("level") == 111
    resp = api.scenes.recall(empty_sid)
    test_artifacts.attach_json("empty_recall", resp)
    assert resp.get("status") == "confirmed", resp
    last = wait_state(short, lambda s: s.get("level") != 111, timeout_s=2.0)
    assert last.get("level") == 111, \
        "empty-scene recall must not move levels: %s" % last
    api.off(short)


@pytest.mark.hil_id("HIL-SCN-06")
@pytest.mark.sniffer
@pytest.mark.optical
def test_group_recall_moves_members_only(api, scenes_supported, vl_bindings,
                                         lamps, scene_matrix_guard,
                                         group_matrix_guard, free_group,
                                         hcl_guard, sniffer, paced,
                                         ops_quiesce, camera_oracle,
                                         wait_state, state_snapshot,
                                         test_artifacts, op_check):
    scene_matrix_guard(SCENE_ID)
    api.cmd_wire(0xFF, 0x50 + SCENE_ID, repeat=2)
    rows = _rows_for(lamps, vl_bindings)
    member_vl = rows[0]["virtual_lamp_id"]
    outsider_vl = rows[1]["virtual_lamp_id"]
    member_short = lamps.by_label[member_vl + 1]
    outsider_short = lamps.by_label[outsider_vl + 1]
    api.scenes.matrix_patch(SCENE_ID, rows)
    api.wait_op(api.scenes.apply(SCENE_ID), timeout_s=60)

    api.groups.join([member_vl], free_group, check=op_check)

    for short in (member_short, outsider_short):
        api.ts(short, {"power": "on", "level": 111})
        assert wait_state(short,
                          lambda s: s.get("level") == 111).get("level") == 111

    with sniffer.window() as win:
        paced(1.0)
        resp = api.scenes.recall(SCENE_ID, group_id=free_group)
        test_artifacts.attach_json("group_recall", resp)
        assert resp.get("status") == "confirmed", resp
        win.expect_frame("GO TO SCENE %d (group %d)" % (SCENE_ID, free_group))
        camera_oracle.assert_on(member_vl + 1, window=win,
                                name="group_member")

    member_state = wait_state(member_short,
                              lambda s: s.get("level") == LEVEL_A)
    test_artifacts.attach_json("member_state", member_state)
    assert member_state.get("level") == LEVEL_A, member_state
    assert member_state.get("last_dapc_source") == "scene", member_state
    outsider_state = wait_state(outsider_short,
                                lambda s: s.get("level") != 111, timeout_s=2.0)
    assert outsider_state.get("level") == 111, \
        "a non-member moved on a group recall: %s" % outsider_state
    api.off_all()


FLEET_SCENE = 13
FLEET_LEVEL = 120


def _fleet_present(api, short):
    r = api.raw((((short << 1) | 1) << 8) | 0x91, expects_backward=True)
    return bool(r.get("success"))


def _fleet_actual_level(api, short):
    r = api.raw((((short << 1) | 1) << 8) | 0xA0, expects_backward=True)
    assert r.get("success"), \
        "QUERY ACTUAL LEVEL unanswered for fleet short %d: %s" % (short, r)
    return r.get("backward_frame")


@pytest.mark.hil_id("HIL-SCN-07")
@pytest.mark.slow
@pytest.mark.sniffer
def test_group_recall_reaches_fleet_members_only(api, gear_sim, sniffer,
                                                 paced, hcl_guard, free_group,
                                                 test_artifacts):
    pre_present = [s for s in range(4, 16) if _fleet_present(api, s)]
    gear_sim.command("enable all")
    time.sleep(1.0)
    candidates = [s for s in range(4, 16) if _fleet_present(api, s)]
    if len(candidates) < 3:
        gear_sim.command("disable all")
        for s in pre_present:
            gear_sim.command("enable %d" % s)
        pytest.skip("need 3 present fleet shorts in 4..15, found %d"
                    % len(candidates))
    members, outsider = candidates[:2], candidates[2]
    touched = candidates[:3]
    group = free_group
    try:
        paced(1.0)
        for short in members:
            r = api.raw((0xA3 << 8) | FLEET_LEVEL)
            assert r.get("success"), r
            api.cmd(short, 0x40 + FLEET_SCENE, repeat=2)
            api.cmd(short, 0x60 + group, repeat=2)
        for short in touched:
            api.raw((short << 1) << 8)
        for short in touched:
            assert _fleet_actual_level(api, short) == 0, \
                "fleet short %d refused to park dark" % short
        with sniffer.window() as win:
            paced(1.0)
            resp = api.scenes.recall(FLEET_SCENE, group_id=group)
            test_artifacts.attach_json("fleet_group_recall", resp)
            assert resp.get("status") == "confirmed", resp
            win.expect_frame("GO TO SCENE %d (group %d)"
                            % (FLEET_SCENE, group))
        levels = {short: _fleet_actual_level(api, short) for short in touched}
        test_artifacts.attach_json("fleet_levels", {
            "members": members, "outsider": outsider, "levels": levels,
        })
        for short in members:
            assert levels[short] == FLEET_LEVEL, \
                "member %d not at scene level: %s" % (short, levels)
        assert levels[outsider] == 0, \
            "outsider %d moved on a group recall: %s" % (outsider, levels)
    finally:
        for short in members:
            api.cmd(short, 0x50 + FLEET_SCENE, repeat=2)
            api.cmd(short, 0x70 + group, repeat=2)
        for short in touched:
            api.raw((short << 1) << 8)
        gear_sim.command("disable all")
        for s in pre_present:
            gear_sim.command("enable %d" % s)
