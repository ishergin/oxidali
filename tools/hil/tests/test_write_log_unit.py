import pytest

from hil import write_log
from hil.write_log import ALL, WriteLog, request_keys

GROUP_ROW = {"virtual_lamp_id": 7, "desired": [False] * 16}


@pytest.mark.parametrize("method,path,body,keys", [
    ("PATCH", "settings/poller", {"enabled": False}, [("settings/poller", {"enabled"})]),
    ("PATCH", "adapters/0", {"name": "x"}, [("adapter/0", {"name"})]),
    ("PATCH", "adapters/0/physical-devices/5", {"name": "x"}, [("device/5", {"name"})]),
    ("DELETE", "adapters/0/physical-devices/5", None, [("device/5", {ALL})]),
    ("POST", "adapters/0/physical-devices/5/write-attributes", {"fade_time_ms": 0},
     [("gear/5", {"fade_time_ms"})]),
    ("PUT", "adapters/0/physical-devices/5/target-state", {"power": "on"},
     [("shown/5", {ALL})]),
    ("PUT", "adapters/0/groups/3/target-state", {"power": "on"}, [("shown/*", {ALL})]),
    ("PATCH", "adapters/0/group-membership-matrix", {"rows": [GROUP_ROW]},
     [("group_matrix/7", {"desired"})]),
    ("PUT", "adapters/0/group-membership-matrix", {"rows": [GROUP_ROW]},
     [("group_matrix/*", {ALL})]),
    ("PATCH", "adapters/0/scenes/3/matrix", {"rows": [{"virtual_lamp_id": 7}]},
     [("scene_matrix/3/7", {"desired"})]),
    ("PATCH", "adapters/0/scenes/3", {"name": "x"}, [("scene/3", {"name"})]),
    ("PATCH", "adapters/0/groups/3", {"ha_entity_enabled": True},
     [("group/3", {"ha_entity_enabled"})]),
    ("POST", "hcl-schedules", {"schedule_id": "hil-x", "enabled": True},
     [("hcl/hil-x", {ALL}), ("shown/*", {ALL})]),
    ("PATCH", "hcl-schedules/owner", {"enabled": False},
     [("hcl/owner", {"enabled"}), ("shown/*", {ALL})]),
    ("DELETE", "hcl-schedules/hil-x/override", None,
     [("hcl_override/hil-x", {ALL}), ("shown/*", {ALL})]),
    ("POST", "rules/hil-a/run", {}, [("shown/*", {ALL})]),
    ("PUT", "rules", {"source": "", "base_revision": 1}, [("rules", {ALL})]),
    ("PATCH", "rules/n%C3%B8tt", {"enabled": True}, [("rule/nøtt", {"enabled"})]),
    ("PUT", "time", {"timezone": "UTC0"}, [("time", {"timezone"}), ("shown/*", {ALL})]),
    ("PUT", "adapters/0/virtual-lamps/7/binding", {"physical_short_address": 2},
     [("vl/7", {"binding"})]),
    ("PATCH", "adapters/0/virtual-lamps/7", {"name": "x"}, [("vl/7", {"name"})]),
    ("PATCH", "policies", {"power_on_level": 254}, [("policies", {"power_on_level"})]),
    ("POST", "policies/apply", None,
     [("gear/*", {"power_on_level", "system_failure_level"})]),
    ("GET", "settings/poller", None, []),
    ("POST", "adapters/0/discovery-runs", {"mode": "refresh_known"}, []),
])
def test_a_request_names_the_resources_and_fields_it_writes(method, path, body, keys):
    assert request_keys(method, path, body) == [(k, frozenset(f)) for k, f in keys]


def test_a_write_log_answers_by_field_and_wildcard_and_survives_a_killed_session(tmp_path):
    path = tmp_path / "production_state_last.writes.json"
    log = WriteLog("t1", "http://dut", path)
    log.note([("gear/*", {"power_on_level"}), ("settings/poller", {"enabled"})])
    for held in (log, WriteLog.load(path, "t1")):
        assert held.changed("gear/5", "power_on_level")
        assert not held.changed("gear/5", "fade_time_ms")
        assert held.changed("settings/poller") and not held.changed("settings/dali")
    assert WriteLog.load(path, "t2") is None
    assert WriteLog.load(tmp_path / "missing.json", "t1") is None
    assert WriteLog.everything("t1", "http://dut").changed("device/9", "name")
    assert write_log.writes_path(tmp_path / "production_state_last.json") == path


def test_only_the_session_log_of_the_same_controller_is_written():
    log = WriteLog("t1", "http://dut")
    write_log.start(log)
    try:
        write_log.note("http://peer", [("settings/poller", {"enabled"})])
        write_log.note("http://dut", [("device/2", {"name"})])
        write_log.note(None, [("gear/2", {ALL})])
    finally:
        write_log.stop()
    write_log.note("http://dut", [("device/3", {"name"})])
    assert sorted(log.touched) == ["device/2", "gear/2"]


def test_an_mqtt_command_may_move_any_lamp_and_a_wildcard_is_no_exact_answer():
    assert write_log.topic_keys("hiltest-dali/hiltest/a0/vl/7/set") == [("shown/*", {ALL})]
    assert write_log.topic_keys("hiltest-dali/hiltest/a0/vl/7/state") == []
    log = WriteLog("t1", "http://dut", touched={"shown/*": {ALL}, "shown/4": {ALL}})
    assert log.changed("shown/5") and not log.changed("shown/5", exact=True)
    assert log.changed("shown/4", exact=True)

