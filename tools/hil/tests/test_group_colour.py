import time

import pytest

CCT_WARM_K = 2700
CCT_COOL_K = 5000
SETTLE_S = 1.0
HCL_TICK_BUDGET_S = 95


def _lamp_id_for(api, short):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        if (lamp.get("binding") or {}).get("physical_short_address") == short:
            return lamp["virtual_lamp_id"]
    return None


def _cct_members(api, capabilities):
    out = []
    for short in api.optical_addrs():
        if _lamp_id_for(api, short) is None:
            continue
        if capabilities.ensure(short, "cct"):
            out.append(short)
    return out


def _activation_census(api, shorts):
    return {s: api.colour_is_activated(s) for s in shorts}


def _levels(api, shorts):
    return {s: (api.state(s).get("state") or {}).get("level") for s in shorts}


@pytest.mark.hil_id("HIL-GRP-08")
@pytest.mark.sniffer
def test_group_colour_only_activates_on_every_member(api, capabilities, lamps,
                                                     vl_bindings, free_group,
                                                     group_matrix_guard,
                                                     ops_quiesce, sniffer, paced,
                                                     wait_state, state_snapshot,
                                                     test_artifacts):
    members = _cct_members(api, capabilities)
    if len(members) < 2:
        pytest.skip("needs at least two bound CCT-capable luminaires (have %r)" % members)

    for short in members:
        api.ts(short, {"power": "on", "level": 120,
                       "color_mode": "cct", "color_temperature_kelvin": CCT_COOL_K})
    for short in members:
        wait_state(short, lambda s: s.get("level") == 120)
    time.sleep(SETTLE_S)
    levels_before = _levels(api, members)

    api.groups.join([_lamp_id_for(api, s) for s in members], free_group)

    colour_only = {"color_mode": "cct", "color_temperature_kelvin": CCT_WARM_K}
    with sniffer.window() as win:
        paced(1.0)
        resp = api.group_ts(free_group, colour_only)
        assert resp.get("status") == "accepted", resp
        win.expect_frame("DT8 SET TEMP COLOUR TEMPERATURE (group %d)" % free_group,
                         resend=lambda: api.group_ts(free_group, colour_only))
        win.expect_frame("DT8 ACTIVATE (group %d)" % free_group)

    time.sleep(SETTLE_S)
    census = _activation_census(api, members)
    levels_after = _levels(api, members)
    test_artifacts.attach_json("group_colour_only", {
        "group": free_group, "members": members,
        "temporary_colour_type_is_mask": census,
        "levels_before": levels_before, "levels_after": levels_after,
    })

    silent = [s for s, v in census.items() if v is None]
    assert not silent, ("gear did not answer QUERY COLOUR VALUE(208) on %r — "
                        "an unanswered probe is not evidence of anything" % silent)
    stale = [s for s, v in census.items() if v is False]
    assert not stale, ("staged colour still pending on %r: TEMPORARY COLOUR TYPE "
                       "is not MASK, so the group command never activated" % stale)
    assert levels_after == levels_before, (levels_before, levels_after)

    api.off_many(members)


@pytest.mark.hil_id("HIL-GRP-09")
@pytest.mark.sniffer
def test_group_colour_leaves_incapable_members_alone(api, capabilities, lamps,
                                                     vl_bindings, free_group,
                                                     group_matrix_guard,
                                                     ops_quiesce, sniffer, paced,
                                                     wait_state, state_snapshot,
                                                     test_artifacts):
    rgb_short = cct_short = None
    for short in api.optical_addrs():
        if _lamp_id_for(api, short) is None:
            continue
        has_rgb = capabilities.ensure(short, "rgb")
        has_cct = capabilities.ensure(short, "cct")
        if has_rgb and rgb_short is None:
            rgb_short = short
        elif has_cct and not has_rgb and cct_short is None:
            cct_short = short
    if rgb_short is None or cct_short is None:
        pytest.skip("needs one RGB-capable and one CCT-only bound luminaire "
                    "(rgb=%r, cct-only=%r)" % (rgb_short, cct_short))

    api.ts(cct_short, {"power": "on", "level": 90,
                       "color_mode": "cct", "color_temperature_kelvin": CCT_COOL_K})
    seeded = wait_state(cct_short,
                        lambda s: s.get("color_temperature_kelvin") not in (None, 0))
    kelvin_before = seeded.get("color_temperature_kelvin")
    api.ts(rgb_short, {"power": "on", "level": 90})
    wait_state(rgb_short, lambda s: s.get("level") == 90)
    time.sleep(SETTLE_S)
    levels_before = _levels(api, [rgb_short, cct_short])

    api.groups.join([_lamp_id_for(api, rgb_short), _lamp_id_for(api, cct_short)],
                    free_group)

    colour_only = {"color_mode": "rgb", "rgb": {"r": 0, "g": 0, "b": 254}}
    with sniffer.window() as win:
        paced(1.0)
        resp = api.group_ts(free_group, colour_only)
        assert resp.get("status") == "accepted", resp
        win.expect_frame("DT8 SET TEMP RGB DIM LEVEL (group %d)" % free_group,
                         resend=lambda: api.group_ts(free_group, colour_only))
        win.expect_frame("DT8 ACTIVATE (group %d)" % free_group)

    time.sleep(SETTLE_S)
    after = api.state(cct_short).get("state") or {}
    levels_after = _levels(api, [rgb_short, cct_short])
    test_artifacts.attach_json("mixed_group_rgb", {
        "rgb_short": rgb_short, "cct_short": cct_short,
        "kelvin_before": kelvin_before, "cct_member_after": after,
        "levels_before": levels_before, "levels_after": levels_after,
    })

    assert levels_after == levels_before, (levels_before, levels_after)
    assert after.get("color_temperature_kelvin") == kelvin_before, (kelvin_before, after)

    api.off_many([rgb_short, cct_short])


@pytest.mark.hil_id("HIL-HCL-10")
@pytest.mark.hil_id("HIL-TS-08")
@pytest.mark.sniffer
def test_hcl_broadcast_colour_point_activates(api, capabilities, lamps,
                                              vl_bindings, clock_guard,
                                              hcl_guard, ops_quiesce, sniffer,
                                              wait_state, state_snapshot,
                                              test_artifacts):
    members = _cct_members(api, capabilities)
    if not members:
        pytest.skip("needs at least one bound CCT-capable luminaire")

    for short in members:
        api.ts(short, {"power": "on", "level": 120,
                       "color_mode": "cct", "color_temperature_kelvin": CCT_COOL_K})
    for short in members:
        wait_state(short, lambda s: s.get("level") == 120)
    time.sleep(SETTLE_S)
    levels_before = _levels(api, members)

    clock_guard(10, 0)
    hcl_guard("hil-bcast-colour")
    schedule = {
        "schedule_id": "hil-bcast-colour",
        "enabled": True,
        "algorithm": "stepped",
        "active_days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
        "location": None,
        "targets": [{"adapter_id": 0, "scope": "broadcast"}],
        "points": [{
            "time_ref": "absolute", "offset_minutes": 9 * 60,
            "level_mode": "none", "level": None,
            "color_temperature_kelvin": CCT_WARM_K,
        }],
    }

    with sniffer.window() as win:
        api.hcl.create(schedule)
        win.expect_frame("DT8 SET TEMP COLOUR TEMPERATURE (broadcast)", timeout_s=HCL_TICK_BUDGET_S)
        win.expect_frame("DT8 ACTIVATE (broadcast)", timeout_s=HCL_TICK_BUDGET_S)

    time.sleep(SETTLE_S)
    census = _activation_census(api, members)
    levels_after = _levels(api, members)
    test_artifacts.attach_json("hcl_broadcast_colour_only", {
        "members": members, "temporary_colour_type_is_mask": census,
        "levels_before": levels_before, "levels_after": levels_after,
    })

    silent = [s for s, v in census.items() if v is None]
    assert not silent, "gear did not answer QUERY COLOUR VALUE(208) on %r" % silent
    stale = [s for s, v in census.items() if v is False]
    assert not stale, ("staged colour still pending on %r after a broadcast "
                       "colour point" % stale)
    assert levels_after == levels_before, (levels_before, levels_after)

    api.off_many(members)
