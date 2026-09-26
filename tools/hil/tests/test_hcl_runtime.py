import time

import pytest

from hil_instruments import _diag_counters
from hil.wait import wait_until

LEVEL_A, LEVEL_B = 90, 200
CCT_K = 3000
TICK_BUDGET_S = 95


def _schedule(schedule_id, group_id, points, enabled=True):
    return {
        "schedule_id": schedule_id,
        "enabled": enabled,
        "algorithm": "stepped",
        "active_days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
        "location": None,
        "targets": [{"adapter_id": 0, "scope": "group", "group_ids": [group_id]}],
        "points": points,
    }


def _point(offset_minutes, level=None, kelvin=None):
    return {
        "time_ref": "absolute",
        "offset_minutes": offset_minutes,
        "level_mode": "absolute" if level is not None else "none",
        "level": level,
        "color_temperature_kelvin": kelvin,
    }


def _wait_ticks(api, extra=1, timeout_s=TICK_BUDGET_S):
    hcl = _diag_counters(api, "hcl")
    return hcl.wait("ticks", hcl.get("ticks") + extra,
                    timeout_s=timeout_s, every_s=2.0)


def _wait_suspended(api, schedule_id, timeout_s=TICK_BUDGET_S):
    override = {}

    def _flagged():
        nonlocal override
        override = api.hcl.override(schedule_id)
        return override.get("suspended")

    wait_until(_flagged, timeout_s, interval_s=2.0)
    return override


def _first_bound(api):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        short = (lamp.get("binding") or {}).get("physical_short_address")
        if short is not None:
            return lamp["virtual_lamp_id"], short
    return None, None


@pytest.mark.hil_id("HIL-HCL-01")
def test_stepped_point_in_the_past_drives_the_group(api, clock_guard,
                                                    hcl_guard, vl_bindings,
                                                    free_group,
                                                    group_matrix_guard,
                                                    ops_quiesce, wait_state,
                                                    state_snapshot,
                                                    test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    clock_guard(10, 0)
    hcl_guard("hil-past")
    api.hcl.create(_schedule("hil-past", free_group, [_point(9 * 60, LEVEL_A)]))

    assert _wait_ticks(api), "the scheduler did not tick"
    state = wait_state(short, lambda s: s.get("level") == LEVEL_A,
                       timeout_s=TICK_BUDGET_S, every_s=2.0)
    test_artifacts.attach_json("state", state)
    assert state.get("level") == LEVEL_A, \
        "the held point never reached the gear: %s" % state
    assert state.get("last_dapc_source") == "group", state
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-02")
def test_crossing_a_point_boundary_drives_the_new_level(api, clock_guard,
                                                        hcl_guard,
                                                        vl_bindings,
                                                        free_group,
                                                        group_matrix_guard,
                                                        ops_quiesce,
                                                        wait_state,
                                                        state_snapshot,
                                                        test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    first, second = 9 * 60, 9 * 60 + 3
    clock_guard(9, 1)
    hcl_guard("hil-cross")
    api.hcl.create(_schedule("hil-cross", free_group,
                             [_point(first, LEVEL_A), _point(second, LEVEL_B)]))

    assert _wait_ticks(api), "the scheduler did not tick on the first point"
    before = wait_state(short, lambda s: s.get("level") == LEVEL_A,
                        timeout_s=TICK_BUDGET_S, every_s=2.0)
    assert before.get("level") == LEVEL_A, \
        "the first point never landed: %s" % before

    clock_guard(9, 4)
    assert _wait_ticks(api), "the scheduler did not tick after the boundary"
    after = wait_state(short, lambda s: s.get("level") == LEVEL_B,
                       timeout_s=TICK_BUDGET_S, every_s=2.0)
    test_artifacts.attach_json("transition",
                               {"before": before, "after": after})
    assert after.get("level") == LEVEL_B, \
        "crossing the boundary did not drive the second point: %s" % after
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-03")
def test_disabled_schedule_drives_nothing(api, clock_guard, hcl_guard,
                                          vl_bindings, free_group,
                                          group_matrix_guard, ops_quiesce,
                                          wait_state, state_snapshot,
                                          test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    parked = 40
    api.ts(short, {"power": "on", "level": parked})
    assert wait_state(short, lambda s: s.get("level") == parked
                      ).get("level") == parked

    clock_guard(10, 0)
    hcl_guard("hil-off")
    api.hcl.create(_schedule("hil-off", free_group,
                             [_point(9 * 60, LEVEL_B)], enabled=False))

    assert _wait_ticks(api, extra=2, timeout_s=TICK_BUDGET_S * 2), \
        "the scheduler did not tick twice"
    state = (api.state(short).get("state") or {})
    test_artifacts.attach_json("state", state)
    assert state.get("level") == parked, \
        "a disabled schedule drove the gear: %s" % state
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-04")
@pytest.mark.needs_capability("cct")
def test_cct_point_reaches_a_colour_capable_group(api, capabilities,
                                                  needs_capability,
                                                  clock_guard, hcl_guard,
                                                  vl_bindings, free_group,
                                                  group_matrix_guard,
                                                  ops_quiesce, wait_state,
                                                  state_snapshot,
                                                  test_artifacts):
    short = capabilities.any_lamp_with("cct")
    if short is None:
        pytest.skip("no cct-capable lamp")
    lamp_id = next((l["virtual_lamp_id"] for l in api.vlamps.list()["virtual_lamps"]
                    if (l.get("binding") or {}).get("physical_short_address") == short),
                   None)
    if lamp_id is None:
        pytest.skip("the cct-capable lamp %d is not bound" % short)
    api.groups.join([lamp_id], free_group)

    clock_guard(10, 0)
    hcl_guard("hil-cct")
    api.hcl.create(_schedule("hil-cct", free_group,
                             [_point(9 * 60, LEVEL_A, kelvin=CCT_K)]))

    assert _wait_ticks(api), "the scheduler did not tick"
    state = wait_state(
        short,
        lambda s: s.get("level") == LEVEL_A
        and s.get("color_temperature_kelvin") not in (None, 0),
        timeout_s=TICK_BUDGET_S, every_s=2.0)
    test_artifacts.attach_json("state", state)
    kelvin = state.get("color_temperature_kelvin")
    assert state.get("level") == LEVEL_A, state
    assert kelvin is not None and abs(kelvin - CCT_K) <= 300, \
        "the scheduled colour temperature did not land: %s" % state
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-05")
@pytest.mark.optical
def test_scheduled_level_is_visible_on_the_fixture(api, lamps, clock_guard,
                                                   hcl_guard, vl_bindings,
                                                   free_group,
                                                   group_matrix_guard,
                                                   ops_quiesce, camera_oracle,
                                                   state_snapshot,
                                                   test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    lamp_id = next((l["virtual_lamp_id"] for l in api.vlamps.list()["virtual_lamps"]
                    if (l.get("binding") or {}).get("physical_short_address") == short),
                   None)
    if lamp_id is None:
        pytest.skip("calibrated lamp %s is not bound" % label)
    api.groups.join([lamp_id], free_group)

    api.ts(short, {"power": "off"})
    camera_oracle.assert_off(label)

    clock_guard(10, 0)
    hcl_guard("hil-optical")
    api.hcl.create(_schedule("hil-optical", free_group,
                             [_point(9 * 60, 254)]))

    assert _wait_ticks(api), "the scheduler did not tick"
    camera_oracle.assert_on(label)
    test_artifacts.attach_json("state", api.state(short).get("state") or {})
    api.off_all()


def _overrides_started(api):
    return int(api.diagnostics()["hcl"]["overrides_started"])


def _overrides_reset(api):
    return int(api.diagnostics()["hcl"]["overrides_reset"])


@pytest.mark.hil_id("HIL-HCL-06")
def test_manual_command_before_the_schedule_ran_does_not_suspend_it(
        api, clock_guard, hcl_guard, vl_bindings, free_group,
        group_matrix_guard, ops_quiesce, wait_state, state_snapshot,
        test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")

    clock_guard(10, 0)
    hcl_guard("hil-ovr-early")
    schedule = _schedule("hil-ovr-early", free_group, [_point(18 * 60, LEVEL_A)])
    schedule["targets"] = [{"adapter_id": 0, "scope": "broadcast"}]
    api.hcl.create(schedule)
    assert _wait_ticks(api), "the scheduler did not tick"
    started_before = _overrides_started(api)

    api.ts(short, {"power": "on", "level": 60})
    assert wait_state(short, lambda s: s.get("level") == 60).get("level") == 60
    assert _wait_ticks(api), "the scheduler did not tick after the manual command"

    override = api.hcl.override("hil-ovr-early")
    test_artifacts.attach_json("override", override)
    assert override["suspended"] is False, \
        "a target the schedule never drove was flagged: %s" % override
    assert _overrides_started(api) == started_before, \
        "the override counter moved for an unapplied target"

    clock_guard(18, 1)
    assert _wait_ticks(api), "the scheduler did not tick past the point"
    state = wait_state(short, lambda s: s.get("level") == LEVEL_A,
                       timeout_s=TICK_BUDGET_S, every_s=2.0)
    test_artifacts.attach_json("state", state)
    assert state.get("level") == LEVEL_A, \
        "the schedule never drove despite holding no flag: %s" % state
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-07")
def test_a_manual_command_after_the_schedule_drove_is_reported_as_an_override(
        api, clock_guard, hcl_guard, vl_bindings, free_group,
        group_matrix_guard, ops_quiesce, wait_state, state_snapshot,
        test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    clock_guard(10, 0)
    hcl_guard("hil-ovr-late")
    api.hcl.create(_schedule("hil-ovr-late", free_group,
                             [_point(9 * 60, LEVEL_A)]))
    assert _wait_ticks(api), "the scheduler did not tick"
    driven = wait_state(short, lambda s: s.get("level") == LEVEL_A,
                        timeout_s=TICK_BUDGET_S, every_s=2.0)
    assert driven.get("level") == LEVEL_A, \
        "the schedule must drive before an override can exist: %s" % driven

    clock_guard(10, 20)
    api.ts(short, {"power": "on", "level": LEVEL_B})
    assert wait_state(short, lambda s: s.get("level") == LEVEL_B,
                      timeout_s=TICK_BUDGET_S, every_s=1.0).get("level") == LEVEL_B

    override = _wait_suspended(api, "hil-ovr-late")
    test_artifacts.attach_json("override", override)
    assert override.get("suspended") is True, \
        "the manual command did not suspend the schedule: %s" % override
    targets = override.get("targets") or []
    assert any(t.get("scope") == "group" and t.get("group_id") == free_group
               for t in targets), \
        "the suspended target is not the group the schedule drives: %s" % override
    assert override.get("since_local_minutes") == 10 * 60 + 20, \
        "since_local_minutes should be the controller's local time: %s" % override
    api.off_all()


@pytest.mark.hil_id("HIL-HCL-08")
def test_resetting_the_override_lets_the_schedule_drive_again(
        api, clock_guard, hcl_guard, vl_bindings, free_group,
        group_matrix_guard, ops_quiesce, wait_state, state_snapshot,
        test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    clock_guard(10, 0)
    hcl_guard("hil-ovr-reset")
    api.hcl.create(_schedule("hil-ovr-reset", free_group,
                             [_point(9 * 60, LEVEL_A)]))
    assert _wait_ticks(api), "the scheduler did not tick"
    assert wait_state(short, lambda s: s.get("level") == LEVEL_A,
                      timeout_s=TICK_BUDGET_S, every_s=2.0).get("level") == LEVEL_A

    api.ts(short, {"power": "on", "level": LEVEL_B})
    _wait_suspended(api, "hil-ovr-reset")
    assert api.hcl.override("hil-ovr-reset").get("suspended") is True, \
        "the schedule was never suspended, so the reset would prove nothing"

    reset_before = _overrides_reset(api)
    api.hcl.clear_override("hil-ovr-reset")
    after = api.hcl.override("hil-ovr-reset")
    test_artifacts.attach_json("override_after_reset", after)
    assert after.get("suspended") is False, \
        "the reset did not lift the flag: %s" % after
    assert _overrides_reset(api) > reset_before, \
        "overrides_reset did not count the lifted flag"

    assert _wait_ticks(api), "the scheduler did not tick after the reset"
    state = wait_state(short, lambda s: s.get("level") == LEVEL_A,
                       timeout_s=TICK_BUDGET_S, every_s=2.0)
    test_artifacts.attach_json("state", state)
    assert state.get("level") == LEVEL_A, \
        "the schedule did not drive again after Resume: %s" % state
    api.off_all()



FULL_GROUPS = "runtime_status,common_102,dt8_color,dt6_led,extended,groups,scenes"
READ_LEAD_S = 6.0
SWEEP_LEVEL = 140


def _dali_counters(api):
    return api.diagnostics()["dali_worker"]


def _wait_tick_edge(api, timeout_s=TICK_BUDGET_S, every_s=0.2):
    hcl = _diag_counters(api, "hcl")
    return hcl.wait("ticks", hcl.get("ticks") + 1,
                    timeout_s=timeout_s, every_s=every_s)


@pytest.mark.hil_id("HIL-HCL-09")
def test_a_slider_queued_behind_a_schedule_sweep_still_wins(
        api, clock_guard, hcl_guard, vl_bindings, free_group,
        group_matrix_guard, ops_quiesce, wait_state, state_snapshot,
        test_artifacts):
    lamp_id, short = _first_bound(api)
    if lamp_id is None:
        pytest.skip("no bound virtual lamp on the rig")
    api.groups.join([lamp_id], free_group)

    clock_guard(10, 0)
    hcl_guard("hil-fifo")

    assert _wait_tick_edge(api), "no tick edge to anchor on"
    edge = time.time()

    api.hcl.create(_schedule("hil-fifo", free_group, [_point(9 * 60, SWEEP_LEVEL)]))

    before = _dali_counters(api)
    time.sleep(max(0.0, 60.0 - READ_LEAD_S - (time.time() - edge)))
    api.attr_read(short, groups=FULL_GROUPS, banks="all")

    assert _wait_tick_edge(api), "the scheduler did not tick"
    api.ts(short, {"power": "on", "level": LEVEL_B})

    state = wait_state(short, lambda s: s.get("level") == LEVEL_B,
                       timeout_s=TICK_BUDGET_S, every_s=1.0)

    override = _wait_suspended(api, "hil-fifo")

    after = _dali_counters(api)
    delta = {name: after.get(name, 0) - before.get(name, 0)
             for name in ("read_attributes_preempted",
                          "read_attributes_transport_aborts",
                          "read_attributes_device_absent")}
    test_artifacts.attach_json("state", state)
    test_artifacts.attach_json("override", override)
    test_artifacts.attach_json("dali_worker_delta", delta)

    assert delta["read_attributes_preempted"] > 0, (
        "INCONCLUSIVE, not a product failure: no read was preempted, so the "
        "sweep and the slider were never in one batch and nothing was ordered "
        "between them. Re-run; if it persists the read is finishing before the "
        "tick and READ_LEAD_S needs raising: %s" % delta)
    assert state.get("level") == LEVEL_B, (
        "the queued schedule sweep overwrote the slider — the two were "
        "reordered by priority: %s" % state)
    assert override.get("suspended") is True, (
        "the schedule never drove before the slider landed, so this run proved "
        "nothing about their order (HIL-HCL-06): %s" % override)
    assert delta["read_attributes_transport_aborts"] == 0, \
        "a yield was miscounted as a transport abort: %s" % delta
    assert delta["read_attributes_device_absent"] == 0, \
        "a yield was miscounted as an absent device: %s" % delta
    api.off_all()
