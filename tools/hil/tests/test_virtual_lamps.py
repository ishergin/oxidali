import pytest


@pytest.mark.hil_id("HIL-VL-01")
def test_binding_and_derived_state(api, vl_bindings, lamps, wait_state,
                                   state_snapshot):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    dto = api.vlamps.get(lid)
    assert (dto.get("binding") or {}).get("physical_short_address") == short
    api.ts(short, {"power": "on", "level": 137})
    last = wait_state(short, lambda s: s.get("level") == 137)
    assert last.get("level") == 137, last
    vl_state = api.vlamps.get(lid)["state"]
    assert vl_state.get("level") == 137, vl_state
    api.off(short)


@pytest.mark.hil_id("HIL-VL-02")
@pytest.mark.sniffer
def test_vl_target_state_reaches_wire(api, vl_bindings, lamps, sniffer, paced,
                                      wait_state, state_snapshot):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    setpoint = {"power": "on", "level": 200}
    with sniffer.window() as win:
        paced(1.0)
        dto = api.vlamps.ts(lid, setpoint)
        assert dto["virtual_lamp_id"] == lid
        win.expect_frame("DAPC short %d -> level 200" % short,
                         resend=lambda: api.vlamps.ts(lid, setpoint))
    last = wait_state(short, lambda s: s.get("level") == 200)
    assert last.get("level") == 200, last
    assert api.vlamps.get(lid)["state"].get("level") == 200
    api.vlamps.ts(lid, {"power": "off"})


@pytest.mark.hil_id("HIL-VL-03")
def test_unbound_target_state_is_noop(api, vl_bindings, binding_guard, lamps,
                                      wait_state, state_snapshot,
                                      test_artifacts):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    api.ts(short, {"power": "on", "level": 90})
    assert wait_state(short, lambda s: s.get("level") == 90).get("level") == 90

    binding_guard(lid)
    api.vlamps.unbind(lid)
    assert api.vlamps.get(lid).get("binding") is None
    resp = api.vlamps.ts(lid, {"power": "on", "level": 222})
    test_artifacts.attach_json("unbound_ts_response", resp)
    last = wait_state(short, lambda s: s.get("level") == 222, timeout_s=2.5)
    assert last.get("level") == 90, \
        "unbound VL target-state must not reach the lamp: %s" % last

    api.vlamps.bind(lid, short)
    assert (api.vlamps.get(lid).get("binding")
            or {}).get("physical_short_address") == short
    api.off(short)


@pytest.mark.hil_id("HIL-VL-04")
def test_vl_patch_metadata_roundtrip(api, vl_bindings, lamps):
    label = lamps.labels()[0]
    lid = label - 1
    before = api.vlamps.get(lid)
    restore_name = before["name"] or ("lamp%d" % label)
    try:
        dto = api.vlamps.patch(lid, {"name": "hil-vl-tmp",
                                     "ha_entity_enabled": True})
        assert dto["name"] == "hil-vl-tmp" and dto["ha_entity_enabled"] is True
        assert api.vlamps.get(lid)["name"] == "hil-vl-tmp"
        listed = {v["virtual_lamp_id"]: v
                  for v in api.vlamps.list()["virtual_lamps"]}
        assert listed[lid]["name"] == "hil-vl-tmp"
    finally:
        api.vlamps.patch(lid, {"name": restore_name,
                               "ha_entity_enabled": before["ha_entity_enabled"]})
