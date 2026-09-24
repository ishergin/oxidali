import pytest

pytestmark = pytest.mark.foreign

LEVEL_A, LEVEL_B = 180, 220


@pytest.mark.hil_id("HIL-FAN-01")
def test_foreign_dapc_projects_with_sniffer_source(api, foreign,
                                                   fanout_supported, lamps,
                                                   sniffer, wait_state,
                                                   state_snapshot,
                                                   test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    with sniffer.window() as win:
        foreign.dapc(short, LEVEL_A)
        win.expect_monitor("ArcPower", direction="tx")
    last = wait_state(short, lambda s: s.get("level") == LEVEL_A)
    test_artifacts.attach_json("state", last)
    assert last.get("level") == LEVEL_A, last
    assert last.get("value_source") == "sniffer", last
    assert last.get("last_dapc_source") == "sniffer", last
    api.off(short)


@pytest.mark.hil_id("HIL-FAN-02")
def test_own_then_foreign_source_flip(api, foreign, fanout_supported, lamps,
                                      wait_state, state_snapshot,
                                      test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    api.ts(short, {"power": "on", "level": 100})
    own = wait_state(short, lambda s: s.get("level") == 100)
    test_artifacts.attach_json("own", own)
    assert own.get("value_source") == "api", own
    assert own.get("last_dapc_source") is None, own

    foreign.dapc(short, LEVEL_B)
    theirs = wait_state(short, lambda s: s.get("level") == LEVEL_B)
    test_artifacts.attach_json("foreign", theirs)
    assert theirs.get("value_source") == "sniffer", theirs
    assert theirs.get("last_dapc_source") == "sniffer", theirs
    api.off(short)


@pytest.mark.hil_id("HIL-FAN-05")
def test_own_group_ts_attributes_group_source(api, fanout_supported, lamps,
                                              vl_bindings, free_group,
                                              group_matrix_guard, ops_quiesce,
                                              wait_state, state_snapshot,
                                              test_artifacts, op_check):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    api.groups.join([lid], free_group, check=op_check)

    api.group_ts(free_group, {"power": "on", "level": 200})
    last = wait_state(short, lambda s: s.get("level") == 200)
    test_artifacts.attach_json("member_state", last)
    assert last.get("level") == 200, last
    assert last.get("value_source") == "api", last
    assert last.get("last_dapc_source") == "group", last
    api.off(short)


@pytest.mark.hil_id("HIL-FAN-03")
def test_foreign_group_dapc_fans_out(api, foreign, fanout_supported, lamps,
                                     vl_bindings, free_group,
                                     group_matrix_guard, ops_quiesce,
                                     wait_state, state_snapshot,
                                     test_artifacts, op_check):
    labels = lamps.labels()
    if len(labels) < 2:
        pytest.skip("needs two lamps: member + non-member")
    member_label, other_label = labels[0], labels[1]
    member_lid = member_label - 1
    member_short = lamps.by_label[member_label]
    other_short = lamps.by_label[other_label]

    api.groups.join([member_lid], free_group, check=op_check)
    api.ts(other_short, {"power": "on", "level": 55})
    assert wait_state(other_short,
                      lambda s: s.get("level") == 55).get("level") == 55

    foreign.group_dapc(free_group, 200)
    member = wait_state(member_short, lambda s: s.get("level") == 200)
    test_artifacts.attach_json("member", member)
    assert member.get("level") == 200, member
    assert member.get("value_source") == "sniffer", member
    assert member.get("last_dapc_source") == "group", member
    untouched = api.state(other_short).get("state") or {}
    assert untouched.get("level") == 55, untouched
    api.off_all()


@pytest.mark.hil_id("HIL-FAN-04")
def test_foreign_cct_write_projects_color(api, foreign, fanout_supported,
                                          capabilities, needs_capability,
                                          wait_state, state_snapshot,
                                          test_artifacts):
    short = capabilities.any_lamp_with("cct")
    if short is None:
        pytest.skip("no cct-capable lamp")
    api.ts(short, {"power": "on", "level": 120})
    before = wait_state(short, lambda s: s.get("level") == 120)

    foreign.set_cct(short, 2700)
    after = wait_state(
        short,
        lambda s: s.get("color_temperature_kelvin") not in
        (None, before.get("color_temperature_kelvin")),
        timeout_s=6.0)
    test_artifacts.attach_json("cct", {"before": before, "after": after})
    kelvin = after.get("color_temperature_kelvin")
    assert kelvin is not None and abs(kelvin - 2700) <= 40, after
    assert after.get("value_source") == "sniffer", after
    assert after.get("last_dapc_source") == before.get("last_dapc_source"), \
        after
    api.off(short)


@pytest.mark.hil_id("HIL-FAN-06")
def test_foreign_scene_recall_projects(api, foreign, fanout_supported,
                                       scenes_supported, vl_bindings, lamps,
                                       scene_matrix_guard, ops_quiesce,
                                       wait_state, state_snapshot,
                                       test_artifacts, op_check):
    scene_id = 12
    scene_matrix_guard(scene_id)
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    api.scenes.matrix_patch(scene_id, [
        {"virtual_lamp_id": lid,
         "desired": {"included": True, "power": "on", "level": 150}}])
    res = api.scenes.apply(scene_id)
    if "operation_id" in res:
        op_check(api.wait_op(res, timeout_s=60))

    foreign.goto_scene(scene_id)
    last = wait_state(short, lambda s: s.get("level") == 150)
    test_artifacts.attach_json("state", last)
    assert last.get("level") == 150, last
    assert last.get("last_dapc_source") == "scene", last
    assert last.get("value_source") == "sniffer", last
    api.off(short)


@pytest.mark.hil_id("HIL-FAN-07")
def test_foreign_dapc_keeps_runtime_status(api, foreign, fanout_supported,
                                           lamps, sniffer, wait_state,
                                           state_snapshot, test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    api.ts(short, {"power": "on", "level": 100})
    wait_state(short, lambda s: s.get("level") == 100)
    api.attr_read_checked(short, groups="runtime_status")
    before = wait_state(short, lambda s: s.get("status") is not None)
    test_artifacts.attach_json("before", before)
    assert before.get("status") is not None, \
        "the read must have committed a status for this test to mean anything"
    raw = before["status"].get("raw")

    with sniffer.window() as win:
        foreign.dapc(short, LEVEL_A)
        win.expect_monitor("ArcPower", direction="tx")
    after = wait_state(short, lambda s: s.get("level") == LEVEL_A)
    test_artifacts.attach_json("after", after)
    assert after.get("level") == LEVEL_A, after
    assert after.get("value_source") == "sniffer", after
    assert after.get("status") is not None, \
        "a DAPC frame observes no status and must not clear one: %r" % (after,)
    assert after["status"].get("raw") == raw, after
    api.off(short)
