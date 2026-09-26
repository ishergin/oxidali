import time

import pytest

from hil.api import ApiError
from hil.wait import wait_until

FLUSH_WINDOW_S = 12


def _wait_flushed(api, read_back, before, timeout_s=45.0):
    if wait_until(lambda: read_back() not in (None, before), timeout_s,
                  interval_s=1.0, desc="slice mutation to be visible"):
        time.sleep(FLUSH_WINDOW_S)
        return True
    return False


@pytest.mark.hil_id("HIL-PERS-02")
@pytest.mark.destructive
@pytest.mark.serial
def test_adapter_slice_survives_reboot(api, dut_reboot, test_artifacts):
    original = api.adapter_info(0)["name"]
    marker = "Slice Store %d" % int(time.time() % 100000)
    try:
        api.adapter_patch({"name": marker}, 0)
        assert _wait_flushed(api, lambda: api.adapter_info(0)["name"], original)

        health = dut_reboot()
        test_artifacts.attach_json("post_reboot_health", health)
        assert api.adapter_info(0)["name"] == marker, "adapter slice did not survive"
    finally:
        api.adapter_patch({"name": original}, 0)


@pytest.mark.hil_id("HIL-PERS-01")
@pytest.mark.destructive
@pytest.mark.serial
def test_config_survives_reboot_runtime_does_not(api, vl_bindings, lamps,
                                                 free_group,
                                                 group_matrix_guard,
                                                 ops_quiesce, dut_reboot,
                                                 wait_state, test_artifacts):
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    name_before = api.state(short).get("name") or ""
    try:
        api.device_patch(short, {"name": "hil-persist"})
        api.groups.join([lid], free_group)
        api.ts(short, {"power": "on", "level": 140})
        assert wait_state(short,
                          lambda s: s.get("level") == 140).get("level") == 140

        flushed = {
            "devices": _wait_flushed(
                api, lambda: api.state(short).get("name"), name_before),
            "groups": _wait_flushed(
                api,
                lambda: next(r for r in api.groups.matrix()["rows"]
                             if r["virtual_lamp_id"] == lid)["desired"][free_group],
                False),
        }
        test_artifacts.attach_json("flush_observed", flushed)
        health = dut_reboot()
        test_artifacts.attach_json("reboot_health", health)

        device = api.state(short)
        assert device.get("name") == "hil-persist", device.get("name")
        binding = (api.vlamps.get(lid).get("binding")
                   or {}).get("physical_short_address")
        assert binding == short, binding

        state = device.get("state") or {}
        test_artifacts.attach_json("runtime_after_reboot", state)
        assert state.get("level") is None, state
        assert state.get("value_source") is None, state

        row_after = next(r for r in api.groups.matrix()["rows"]
                         if r["virtual_lamp_id"] == lid)
        test_artifacts.attach_json("matrix_row_after_reboot", row_after)
        assert row_after["desired"][free_group] is True, row_after
        assert row_after["applied"][free_group] is True, row_after
    finally:
        api.device_patch(short, {"name": name_before or None})
        try:
            api.cmd(short, 0x70 + free_group, repeat=2)
        except ApiError:
            pass


@pytest.mark.hil_id("HIL-PERS-03")
@pytest.mark.destructive
@pytest.mark.serial
def test_operations_do_not_survive_reboot(api, lamps, dut_reboot,
                                          test_artifacts):
    short = lamps.by_label[lamps.labels()[0]]
    view = api.attr_read_checked(short, groups="runtime_status")
    op = {"operation_id": view["operation_id"]}
    dut_reboot()
    status, body = api.raw_request("GET", "operations/%s" % op["operation_id"])
    test_artifacts.attach_json("after_reboot", {"status": status, "body": body})
    assert status == 404, (status, body)
    assert op["operation_id"] not in api.operations()


@pytest.mark.hil_id("HIL-PERS-04")
@pytest.mark.destructive
@pytest.mark.serial
@pytest.mark.sniffer
def test_scene_survives_in_gear_across_reboot(api, scenes_supported,
                                              vl_bindings, lamps,
                                              scene_matrix_guard, ops_quiesce,
                                              sniffer, paced, dut_reboot,
                                              wait_state, state_snapshot,
                                              test_artifacts):
    scene_id = 12
    scene_matrix_guard(scene_id)
    label = lamps.labels()[0]
    lid, short = label - 1, lamps.by_label[label]
    api.scenes.matrix_patch(scene_id, [
        {"virtual_lamp_id": lid,
         "desired": {"included": True, "power": "on", "level": 150}}])
    res = api.scenes.apply(scene_id)
    if "operation_id" in res:
        assert api.wait_op(res, timeout_s=60)["status"] == "succeeded"

    time.sleep(FLUSH_WINDOW_S)
    dut_reboot()
    with sniffer.window() as win:
        paced(1.0)
        resp = api.scenes.recall(scene_id)
        assert resp.get("status") == "confirmed", resp
        win.expect_frame("GO TO SCENE %d" % scene_id)
    last = wait_state(short, lambda s: s.get("level") == 150)
    test_artifacts.attach_json("recalled_state", last)
    assert last.get("level") == 150, last
    api.off(short)


@pytest.mark.hil_id("HIL-PERS-05")
@pytest.mark.destructive
@pytest.mark.serial
def test_hcl_schedule_survives_reboot(api, dut_reboot, test_artifacts):
    schedule_id = "hil-pers-%d" % int(time.time() % 100000)
    body = {
        "schedule_id": schedule_id,
        "enabled": False,
        "algorithm": "interpolated",
        "active_days": ["mon", "wed", "fri"],
        "location": {"latitude_deg": 55.7558, "longitude_deg": 37.6173},
        "targets": [{"adapter_id": 0, "scope": "group", "group_ids": [1, 5]}],
        "points": [
            {"time_ref": "absolute", "offset_minutes": 360, "level_mode": "absolute",
             "level": 80, "color_temperature_kelvin": 2700},
            {"time_ref": "sunset", "offset_minutes": -30, "level_mode": "last_active",
             "level": None, "color_temperature_kelvin": 2200},
        ],
    }
    try:
        created = api.hcl.create(body)
        assert created["schedule_id"] == schedule_id, created
        assert _wait_flushed(api, lambda: api.hcl.get(schedule_id), None)

        health = dut_reboot()
        test_artifacts.attach_json("post_reboot_health", health)

        after = api.hcl.get(schedule_id)
        test_artifacts.attach_json("hydrated_schedule", after)
        for key in ("enabled", "algorithm", "active_days", "location",
                    "targets", "points"):
            assert after[key] == body[key], (key, after[key], body[key])
    finally:
        try:
            api.hcl.delete(schedule_id)
        except ApiError:
            pass
