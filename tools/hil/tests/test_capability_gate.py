import pytest

LEVEL = 160
CCT_K = 2700
RGB_TARGET = {"r": 0, "g": 0, "b": 255}


def _lamp_id_for(api, short):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        if (lamp.get("binding") or {}).get("physical_short_address") == short:
            return lamp["virtual_lamp_id"]
    return None


def _split_by_rgb(api, capabilities):
    rgb_short = cct_short = None
    for device in api.devices()["physical_devices"]:
        short = device["short_address"]
        if _lamp_id_for(api, short) is None:
            continue
        has_rgb = capabilities.ensure(short, "rgb")
        has_cct = capabilities.ensure(short, "cct")
        if has_rgb and rgb_short is None:
            rgb_short = short
        elif has_cct and not has_rgb and cct_short is None:
            cct_short = short
    return rgb_short, cct_short


@pytest.mark.hil_id("HIL-CAP-01")
def test_group_rgb_is_stripped_for_a_cct_only_member(api, capabilities,
                                                     vl_bindings, free_group,
                                                     group_matrix_guard,
                                                     ops_quiesce, wait_state,
                                                     state_snapshot,
                                                     test_artifacts):
    rgb_short, cct_short = _split_by_rgb(api, capabilities)
    if rgb_short is None or cct_short is None:
        pytest.skip("needs one RGB-capable and one CCT-only bound lamp "
                    "(rig has rgb=%r, cct-only=%r)" % (rgb_short, cct_short))

    api.ts(cct_short, {"power": "on", "level": 80,
                       "color_mode": "cct", "color_temperature_kelvin": CCT_K})
    seeded = wait_state(cct_short,
                        lambda s: s.get("color_temperature_kelvin") not in (None, 0))
    kelvin_before = seeded.get("color_temperature_kelvin")
    assert kelvin_before is not None, seeded

    api.groups.join([_lamp_id_for(api, rgb_short), _lamp_id_for(api, cct_short)],
                    free_group)
    api.group_ts(free_group, {"power": "on", "level": LEVEL,
                              "color_mode": "rgb", "rgb": RGB_TARGET})

    rgb_state = wait_state(rgb_short, lambda s: s.get("level") == LEVEL)
    cct_state = wait_state(cct_short, lambda s: s.get("level") == LEVEL)
    test_artifacts.attach_json("rgb_member", rgb_state)
    test_artifacts.attach_json("cct_member",
                               {"before": seeded, "after": cct_state})

    assert rgb_state.get("level") == LEVEL, rgb_state
    assert cct_state.get("level") == LEVEL, cct_state
    assert rgb_state.get("color_mode") == "rgb", rgb_state
    assert cct_state.get("color_mode") != "rgb", \
        "RGB was projected onto a CCT-only lamp: %s" % cct_state
    assert cct_state.get("rgb") in (None, {}), \
        "an RGB triple reached a CCT-only lamp: %s" % cct_state
    assert cct_state.get("color_temperature_kelvin") == kelvin_before, \
        "the CCT-only lamp's stored kelvin was clobbered: %r -> %r" % (
            kelvin_before, cct_state.get("color_temperature_kelvin"))
    api.off_all()


@pytest.mark.hil_id("HIL-CAP-02")
def test_group_cct_reaches_every_cct_capable_member(api, capabilities,
                                                    vl_bindings, free_group,
                                                    group_matrix_guard,
                                                    ops_quiesce, wait_state,
                                                    state_snapshot,
                                                    test_artifacts):
    rgb_short, cct_short = _split_by_rgb(api, capabilities)
    if rgb_short is None or cct_short is None:
        pytest.skip("needs one RGB-capable and one CCT-only bound lamp")

    api.groups.join([_lamp_id_for(api, rgb_short), _lamp_id_for(api, cct_short)],
                    free_group)
    api.group_ts(free_group, {"power": "on", "level": LEVEL,
                              "color_mode": "cct",
                              "color_temperature_kelvin": CCT_K})

    def landed(state):
        kelvin = state.get("color_temperature_kelvin")
        return kelvin is not None and abs(kelvin - CCT_K) <= 200

    both = {}
    for short in (rgb_short, cct_short):
        both[short] = wait_state(short, landed, timeout_s=8.0)
    test_artifacts.attach_json("members", both)
    for short, state in both.items():
        assert landed(state), \
            "CCT did not reach capable member %d: %s" % (short, state)
        assert state.get("color_mode") == "cct", state
    api.off_all()


@pytest.mark.hil_id("HIL-CAP-03")
def test_unscanned_member_fails_open(api, capabilities, vl_bindings,
                                     free_group, group_matrix_guard,
                                     ops_quiesce, wait_state, state_snapshot,
                                     test_artifacts):
    unscanned = None
    for device in api.devices()["physical_devices"]:
        short = device["short_address"]
        if _lamp_id_for(api, short) is None:
            continue
        flags = device.get("capabilities") or {}
        if not (flags.get("cct") or flags.get("xy") or flags.get("rgb")):
            unscanned = short
            break
    if unscanned is None:
        pytest.skip("every bound device already has confirmed colour flags — "
                    "needs a device this controller has never read")

    lamp_id = _lamp_id_for(api, unscanned)
    api.groups.join([lamp_id], free_group)
    api.group_ts(free_group, {"power": "on", "level": LEVEL,
                              "color_mode": "cct",
                              "color_temperature_kelvin": CCT_K})

    state = wait_state(unscanned,
                       lambda s: s.get("color_temperature_kelvin") not in (None, 0),
                       timeout_s=8.0)
    test_artifacts.attach_json("unscanned_member", state)
    assert state.get("color_temperature_kelvin") is not None, \
        "colour was withheld from a device with no confirmed flags: %s" % state
    api.off_all()
