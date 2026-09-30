import dataclasses
import json
import os
import re
import stat
import types

import pytest

import hil.api
from hil import prod_state, virtual_gear, write_log
from hil.config import load as load_config
from hil.write_log import ALL, WriteLog, request_keys

GROUP_ROW = {"virtual_lamp_id": 7, "desired": [False] * 16}


@pytest.mark.parametrize("method,path,body,keys", [
    ("PATCH", "settings/poller", {"enabled": False}, [("settings/poller", {"enabled"})]),
    ("PATCH", "adapters/0", {"name": "x"}, [("adapter/0", {"name"})]),
    ("PATCH", "adapters/0/physical-devices/5", {"name": "x"}, [("device/5", {"name"})]),
    ("DELETE", "adapters/0/physical-devices/5", None,
     [("device/5", {ALL}), ("vl/*", {"binding"}), ("group_matrix/*", {ALL}),
      ("scene_matrix/*", {ALL})]),
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
    ("POST", "adapters/0/discovery-runs", {"mode": "refresh_known"},
     [("gear/*", {"power_on_level", "system_failure_level"})]),
    ("POST", "redundancy/switchover", {}, [("shown/*", {ALL})]),
    ("POST", "firmware/updates", {"url": "http://x/app.bin"}, [("shown/*", {ALL})]),
    ("PATCH", "settings/dali", {"application_active": True},
     [("settings/dali", {"application_active"}), ("shown/*", {ALL})]),
    ("PATCH", "settings/dali", {"application_active": False},
     [("settings/dali", {"application_active"})]),
    ("POST", "adapters/0/input-devices/scan", {}, []),
])
def test_a_request_names_the_resources_and_fields_it_writes(method, path, body, keys):
    assert request_keys(method, path, body) == [(k, frozenset(f)) for k, f in keys]


def test_a_write_log_answers_by_field_and_wildcard_and_survives_a_killed_session(tmp_path):
    path = tmp_path / "production_state_last.writes.json"
    log = WriteLog("t1", "http://dut", path)
    log.note([("gear/*", {"power_on_level"}), ("settings/poller", {"enabled"})])
    for held in (log, WriteLog.load(path, "t1", "http://dut")):
        assert held.changed("gear/5", "power_on_level")
        assert not held.changed("gear/5", "fade_time_ms")
        assert held.changed("settings/poller") and not held.changed("settings/dali")
    assert WriteLog.load(tmp_path / "missing.json", "t1", "http://dut") is None
    assert WriteLog.everything("t1", "http://dut").changed("device/9", "name")
    assert write_log.writes_path(tmp_path / "production_state_last.json") == path


def test_only_the_session_log_of_the_same_controller_is_written():
    log = WriteLog("t1", "http://dut")
    with write_log.recording(log):
        write_log.note("http://peer", [("settings/poller", {"enabled"})])
        write_log.note("http://dut", [("device/2", {"name"})])
        write_log.note(None, [("gear/2", {ALL})])
    write_log.note("http://dut", [("device/3", {"name"})])
    assert sorted(log.touched) == ["device/2", "gear/2"]


def test_a_nested_recording_hands_the_session_log_back_when_it_ends():
    session, unit = WriteLog("t1", "http://dut"), WriteLog("t2", "http://dut")
    with write_log.recording(session):
        with write_log.recording(unit):
            write_log.note("http://dut", [("device/2", {"name"})])
        write_log.note("http://dut", [("device/3", {"name"})])
    assert sorted(unit.touched) == ["device/2"]
    assert sorted(session.touched) == ["device/3"]


@pytest.fixture(scope="module")
def outer_session_log():
    log = WriteLog("session", "http://dut")
    with write_log.recording(log):
        yield log


def test_a_unit_test_writes_nothing_into_the_session_log(outer_session_log):
    write_log.note("http://dut", [("device/5", {"name"})])
    with write_log.recording(WriteLog("t2", "http://dut")):
        write_log.note("http://dut", [("device/6", {"name"})])
    write_log.note(None, [("gear/7", {ALL})])
    assert outer_session_log.touched == {}


def test_an_mqtt_command_may_move_any_lamp_and_a_wildcard_is_no_exact_answer():
    assert write_log.topic_keys("hiltest-dali/hiltest/a0/vl/7/set") == [("shown/*", {ALL})]
    assert write_log.topic_keys("hiltest-dali/hiltest/a0/vl/7/state") == []
    log = WriteLog("t1", "http://dut", touched={"shown/*": {ALL}, "shown/4": {ALL}})
    assert log.changed("shown/5") and not log.changed("shown/5", exact=True)
    assert log.changed("shown/4", exact=True)


class _NoRules:
    def request(self, method, url, json=None, timeout=None):
        return types.SimpleNamespace(status_code=200, content=b"{}",
                                     json=lambda: {"rules": {"rules": []}})

    def close(self):
        pass


def test_a_reboot_the_toolkit_provokes_may_move_any_lamp():
    client = hil.api.Client(dataclasses.replace(load_config(), lamps_read_only=False))
    client.http = _NoRules()
    log = WriteLog("t1", client.base)
    with write_log.recording(log):
        with client.expect_reboot():
            pass
    assert log.changed("shown/5") and not log.changed("gear/5")


@pytest.mark.parametrize("mine,theirs,meet", [
    ({"settings/poller": {"interval_ms"}}, {"settings/poller": {"interval_ms", "enabled"}}, True),
    ({"settings/poller": {"interval_ms"}}, {"settings/poller": {"enabled"}}, False),
    ({"gear/*": {"power_on_level"}}, {"gear/3": {"power_on_level"}}, True),
    ({"gear/3": {"fade_time_ms"}}, {"gear/*": {"power_on_level"}}, False),
    ({"shown/2": {ALL}}, {"shown/*": {ALL}}, True),
    ({"device/9": {"name"}}, {ALL: {ALL}}, True),
    ({"device/9": {"name"}}, {"device/10": {"name"}}, False),
])
def test_two_logs_meet_where_a_key_and_a_field_of_one_cover_the_other(mine, theirs, meet):
    ours, other = WriteLog("a", "b", touched=mine), WriteLog("c", "b", touched=theirs)
    assert ours.overlaps(other) is meet and other.overlaps(ours) is meet



@pytest.mark.parametrize("text,why", [
    ("{not json", "unreadable"),
    ('["t1"]', "no key-to-fields map"),
    ('{"session": "t1", "base": "http://dut"}', "no key-to-fields map"),
    ('{"session": "t1", "base": "http://dut", "touched": {"settings/poller": "enabled"}}',
     "not a list of field names"),
    ('{"session": "t0", "base": "http://dut", "touched": {}}', "session 't0'"),
    ('{"session": "t1", "base": "http://other", "touched": {}}', "http://other, not http://dut"),
])
def test_a_log_of_the_wrong_shape_or_another_controller_is_refused_not_read_as_no_writes(
        tmp_path, text, why):
    path = tmp_path / "production_state-1.writes.json"
    path.write_text(text)
    with pytest.raises(write_log.WriteLogError, match=re.escape(why)):
        WriteLog.load(path, "t1", "http://dut")


def test_every_state_file_reaches_the_disk_before_its_name_does(monkeypatch, tmp_path):
    events, real_fsync, real_replace = [], os.fsync, os.replace

    def fsync(fd):
        events.append("fsync " + ("folder" if stat.S_ISDIR(os.fstat(fd).st_mode) else "file"))
        real_fsync(fd)

    def replace(source, target):
        events.append("rename")
        real_replace(source, target)

    monkeypatch.setattr(os, "fsync", fsync)
    monkeypatch.setattr(os, "replace", replace)
    for save in (lambda path: prod_state.save({"taken_at": "t1"}, path),
                 lambda path: WriteLog("t1", "http://dut", path).save(),
                 lambda path: virtual_gear.Ledger(path).update(step="scan")):
        events.clear()
        path = tmp_path / "state.json"
        save(path)
        assert events == ["fsync file", "rename", "fsync folder"]
        assert json.loads(path.read_text()) and not (tmp_path / "state.json.tmp").exists()
