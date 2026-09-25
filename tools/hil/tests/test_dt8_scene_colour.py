import pytest

from hil.wait import wait_until

SCENE_ID = 9
AUDIT_LEVEL = 90
REMOVE_FROM_SCENE = 0x50


def _mirek(kelvin):
    return round(1_000_000 / kelvin)


def _range_mirek(state):
    rng = state.get("color_temperature_range") or {}
    if not rng.get("min_kelvin") or not rng.get("max_kelvin"):
        return None, None
    return _mirek(rng["max_kelvin"]), _mirek(rng["min_kelvin"])


def _active_tc_mirek(state):
    dt8 = (state.get("attributes") or {}).get("dt8_color") or {}
    slot = dt8.get("color_value_2") or {}
    return slot.get("value")


def _cct_lamp(api):
    allowed = api.cfg.lamp_short_set()
    short = next((d["short_address"]
                  for d in api.devices()["physical_devices"]
                  if d["short_address"] in allowed
                  and (d.get("capabilities") or {}).get("cct")), None)
    if short is None:
        pytest.skip("no cct-capable device in HIL_LAMP_SHORTS on the bus")
    vl = next((v["virtual_lamp_id"]
               for v in api.vlamps.list()["virtual_lamps"]
               if (v.get("binding") or {}).get("physical_short_address") == short),
              None)
    if vl is None:
        pytest.skip("cct device has no bound virtual lamp")
    return short, vl


@pytest.mark.hil_id("HIL-DT8L-01")
@pytest.mark.sniffer
def test_tc_limit_write_roundtrip_and_snap(api, sniffer, paced,
                                           state_snapshot, test_artifacts):
    short, _ = _cct_lamp(api)
    api.attr_read_checked(short, groups="dt8_color")
    orig_coolest, orig_warmest = _range_mirek(api.state(short))
    if orig_coolest is None:
        pytest.skip("device reports no Tc range to narrow and restore")
    if orig_warmest - orig_coolest < 60:
        pytest.skip("Tc range too narrow to narrow further and still restore")
    narrowed = orig_warmest - 40

    try:
        api.ts(short, {"power": "on", "level": 140, "color_mode": "cct",
                       "color_temperature_kelvin": 1_000_000 // orig_warmest})
        paced(1.0)
        op = api.write_attrs(short, {"tc_warmest_mirek": narrowed})
        assert op.get("type") == "attribute_write", op
        view = api.wait_op(op)
        test_artifacts.attach_json("tc_limit_write_op", view)
        assert view["status"] == "succeeded", view

        after = api.state(short)
        got_coolest, got_warmest = _range_mirek(after)
        test_artifacts.attach_json("tc_limit_roundtrip", {
            "orig": [orig_coolest, orig_warmest],
            "narrowed_to": narrowed,
            "readback": [got_coolest, got_warmest],
        })
        assert abs(got_warmest - narrowed) <= 1, (got_warmest, narrowed)
        assert abs(got_coolest - orig_coolest) <= 1, (got_coolest, orig_coolest)

        api.attr_read_checked(short, groups="dt8_color")
        active = _active_tc_mirek(api.attributes(short, ["dt8_color"]))
        test_artifacts.attach_json("tc_after_snap", {"active_mirek": active})
        assert active is not None, "no active Tc readback after the limit write"
        assert abs(active - narrowed) <= 1, (active, narrowed)
    finally:
        restore = api.write_attrs(short, {"tc_coolest_mirek": orig_coolest,
                                          "tc_warmest_mirek": orig_warmest})
        api.wait_op(restore)


@pytest.mark.hil_id("HIL-SCNC-01")
@pytest.mark.sniffer
def test_scene_colour_audit_reads_stored_not_active(api, scene_matrix_guard,
                                                    sniffer, paced, ops_quiesce,
                                                    state_snapshot,
                                                    test_artifacts):
    short, vl = _cct_lamp(api)
    api.attr_read_checked(short, groups="dt8_color")
    coolest, warmest = _range_mirek(api.state(short))
    if coolest is None or warmest - coolest < 80:
        pytest.skip("need a usable Tc range to hold two distinct colours")
    scene_mirek = warmest - 10
    active_mirek = coolest + 10
    scene_kelvin = 1_000_000 // scene_mirek

    scene_matrix_guard(SCENE_ID)
    api.cmd(short, REMOVE_FROM_SCENE + SCENE_ID, repeat=2)
    api.scenes.matrix_patch(SCENE_ID, [{
        "virtual_lamp_id": vl,
        "desired": {"included": True, "power": "on", "level": AUDIT_LEVEL,
                    "color_mode": "cct",
                    "color_temperature_kelvin": scene_kelvin},
    }])
    view = api.wait_op(api.scenes.apply(SCENE_ID), timeout_s=60)
    test_artifacts.attach_json("scene_apply_op", view)
    assert view["status"] == "succeeded", view

    api.ts(short, {"power": "on", "level": 140, "color_mode": "cct",
                   "color_temperature_kelvin": 1_000_000 // active_mirek})

    with sniffer.window() as win:
        paced(1.0)
        api.attr_read_checked(short, groups="scene_colours")
        win.expect_frame("QUERY SCENE LEVEL %d" % SCENE_ID)
        win.expect_quiet("GO TO SCENE", settle_s=1.0)

    row = {}

    def _kelvin_projected():
        nonlocal row
        row = next(r for r in api.scenes.matrix(SCENE_ID)["rows"]
                   if r["virtual_lamp_id"] == vl)
        return (row["applied"].get("color_temperature_kelvin") or 0) > 0

    wait_until(_kelvin_projected, 3.0, interval_s=0.3)
    test_artifacts.attach_json("audited_row", row)
    applied = row["applied"]
    assert applied.get("color_mode") == "cct", row
    got = applied.get("color_temperature_kelvin")
    assert got is not None and abs(_mirek(got) - scene_mirek) <= 1, (
        got, {"scene_mirek": scene_mirek, "active_mirek": active_mirek})
    assert abs(_mirek(got) - active_mirek) > 5, (
        "audit returned the ACTIVE colour — the §12 transaction failed", got)
