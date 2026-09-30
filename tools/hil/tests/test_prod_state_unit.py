import copy
import re
import inspect
import types

import pytest

import hil_session
import hil_session_guards
import hil_test_guards
import test_attributes
import test_policies
from hil import api as api_mod
from hil import prod_state, write_log
from hil.lamp_guard import GROUP_TARGET, LampNotAllowed, http_rule


def _snap():
    device = {"record": {"name": "Балкон.Стол1", "device_type_source": "discovered",
                         "device_type_effective": "dt8_color"},
              "state": {"power": "on", "level": 120, "color_mode": "cct",
                        "color_temperature_kelvin": 2702, "rgb": None},
              "config": {"fade_time_ms": 0, "power_on_level": 254},
              "groups": 0x0002, "scenes": [255] * 16}
    settings = {"poller": {"enabled": False, "interval_ms": 5000},
                "dali": {"application_active": True},
                "redundancy": {"enabled": True, "role": "primary"},
                "ha": {"enabled": True, "controller_id": "dali-e0d190"}}
    return {"devices": {"10": device}, "settings": settings, "timezone": "MSK-3",
            "adapter": {"enabled": True}, "rules": {"source": "rule a {}"},
            "hcl": [{"schedule_id": "moscow-cct", "enabled": True}],
            "vl": {"virtual_lamps": [{"virtual_lamp_id": 10, "name": "Стол1",
                                      "binding": {"physical_short_address": 10}}]},
            "groups": [{"group_id": 1, "name": "Балкон.Стол"}],
            "group_matrix": {"rows": []}, "scenes": []}


def test_an_unchanged_installation_has_no_residual():
    assert prod_state.diff(_snap(), _snap()) == []


def test_every_layer_that_moved_is_named():
    after = copy.deepcopy(_snap())
    after["settings"]["ha"]["controller_id"] = "hiltest"
    after["hcl"][0]["enabled"] = False
    after["vl"]["virtual_lamps"][0]["binding"] = {"physical_short_address": 11}
    dev = after["devices"]["10"]
    dev["groups"] = 0x0003
    dev["config"]["fade_time_ms"] = 700
    dev["state"]["level"] = 40
    lines = "\n".join(prod_state.diff(_snap(), after))
    for needle in ("settings/ha.controller_id", "hcl differs", "VL10 binding",
                   "SA10 gear groups", "SA10 gear config", "SA10 shows"):
        assert needle in lines, (needle, lines)


def test_a_device_missing_from_the_registry_is_a_residual():
    after = copy.deepcopy(_snap())
    after["devices"] = {}
    assert prod_state.diff(_snap(), after) == ["SA10 is gone from the registry"]


def test_an_on_lamp_gets_level_and_colour_back():
    state = _snap()["devices"]["10"]["state"]
    assert prod_state._setpoint(state) == {
        "power": "on", "level": 120, "color_mode": "cct",
        "color_temperature_kelvin": 2702}


def test_an_off_lamp_gets_its_colour_staged_without_a_level():
    state = dict(_snap()["devices"]["10"]["state"], power="off", level=0)
    body = prod_state._setpoint(state)
    assert body["power"] == "off" and "level" not in body
    assert body["color_temperature_kelvin"] == 2702


def test_an_override_is_cleared_or_restored_by_what_the_snapshot_says():
    discovered = {"device_type_source": "discovered", "device_type_effective": "dt8_color"}
    override = {"device_type_source": "manual_override", "device_type_effective": "dt6_led"}
    assert prod_state._override_patch(discovered, override, "device_type") == \
        {"device_type_override": None}
    assert prod_state._override_patch(override, discovered, "device_type") == \
        {"device_type_override": "dt6_led"}
    assert prod_state._override_patch(override, override, "device_type") == {}


class _Wire:
    def __init__(self):
        self.sent, self.dtr0, self.scenes = [], None, {}

    def cmd(self, short, opcode, repeat=1):
        self.sent.append(("cmd", short, opcode, repeat))
        if prod_state.SET_SCENE <= opcode < prod_state.SET_SCENE + 16:
            self.scenes[opcode - prod_state.SET_SCENE] = self.dtr0

    def raw(self, frame, expects_backward=False):
        if frame >> 8 == prod_state.DTR0:
            self.dtr0 = frame & 0xFF
            return {"success": True}
        self.sent.append(("query", frame))
        data = frame & 0xFF
        if prod_state.QUERY_SCENE_LEVEL <= data < prod_state.QUERY_SCENE_LEVEL + 16:
            return {"success": True,
                    "backward_frame": self.scenes.get(data - prod_state.QUERY_SCENE_LEVEL)}
        return {"success": True, "backward_frame": self.dtr0}


def test_group_repair_sends_only_the_bits_that_differ_send_twice(monkeypatch):
    monkeypatch.setattr(prod_state, "FRAME_PACE_S", 0)
    wire = _Wire()
    prod_state._repair_groups(wire, 10, want=0b0101, have=0b0110, log=lambda _: None)
    assert wire.sent == [("cmd", 10, prod_state.ADD_TO_GROUP + 0, 2),
                         ("cmd", 10, prod_state.REMOVE_FROM_GROUP + 1, 2)]


def test_scene_repair_proves_dtr0_before_set_scene_and_removes_a_mask(monkeypatch):
    monkeypatch.setattr(prod_state, "FRAME_PACE_S", 0)
    wire = _Wire()
    want = [255] * 16
    have = [255] * 16
    want[3], have[5] = 100, 100
    prod_state._repair_scenes(wire, 1, want, have, log=lambda _: None)
    assert wire.sent[0][0] == "query"
    assert ("cmd", 1, prod_state.SET_SCENE + 3, 2) in wire.sent
    assert ("cmd", 1, prod_state.REMOVE_FROM_SCENE + 5, 2) in wire.sent
    assert wire.sent.index(("cmd", 1, prod_state.SET_SCENE + 3, 2)) > 0


def test_a_lamp_nobody_may_drive_is_reported_not_residual():
    after = copy.deepcopy(_snap())
    after["devices"]["10"]["state"]["level"] = 40
    assert any("SA10 shows" in line for line in prod_state.diff(_snap(), after))
    assert prod_state.diff(_snap(), after, shown_shorts=set()) == []
    assert prod_state.diff(_snap(), after, shown_shorts={11}) == []


def test_a_new_hcl_override_is_a_residual():
    before = dict(_snap(), hcl_overrides={"moscow-cct": False})
    after = dict(_snap(), hcl_overrides={"moscow-cct": True})
    assert any("hcl overrides" in line for line in prod_state.diff(before, after))


def test_a_scene_level_that_did_not_land_is_set_again(monkeypatch):
    monkeypatch.setattr(prod_state, "FRAME_PACE_S", 0)
    wire = _Wire()
    real_cmd = wire.cmd
    spoiled = []

    def cmd(short, opcode, repeat=1):
        if opcode == prod_state.SET_SCENE + 2 and not spoiled:
            spoiled.append(True)
            wire.dtr0 = 7
        real_cmd(short, opcode, repeat)

    wire.cmd = cmd
    assert prod_state._set_scene_verified(wire, 1, 2, 100)
    assert wire.scenes[2] == 100
    assert sum(1 for op in wire.sent if op[:3] == ("cmd", 1, prod_state.SET_SCENE + 2)) == 2


def test_what_an_enabled_schedule_drives_is_not_a_residual():
    before = copy.deepcopy(_snap())
    before["hcl"] = [{"schedule_id": "moscow-cct", "enabled": True,
                      "targets": [{"adapter_id": 0, "scope": "group", "group_ids": [1]}],
                      "points": [{"level_mode": "none", "color_temperature_kelvin": 5000}]}]
    before["group_matrix"] = {"rows": [{"virtual_lamp_id": 10,
                                        "applied": [False, True] + [False] * 14}]}
    after = copy.deepcopy(before)
    after["devices"]["10"]["state"]["color_temperature_kelvin"] = 2950
    assert prod_state.diff(before, after) == []
    after["devices"]["10"]["state"]["power"] = "off"
    assert any("SA10 shows" in line for line in prod_state.diff(before, after)), \
        "the schedule drives colour only; power is still the session's"


class _FadingGear(_Wire):
    def __init__(self, fade_ms):
        super().__init__()
        self.fade_ms = fade_ms
        self.writes = []

    def attributes(self, short, sections=None):
        return {"attributes": {"common_102": {"fade_time_ms": {"value": self.fade_ms}}}}

    def write_attrs(self, short, body):
        self.writes.append((short, dict(body)))
        self.fade_ms = body.get("fade_time_ms", self.fade_ms)
        return {"operation_id": "attr-write-%d" % len(self.writes)}

    def wait_op(self, op):
        return {"operation_id": op["operation_id"], "status": "succeeded"}


def test_fast_fade_comes_after_the_snapshot_and_the_restore_takes_it_back():
    prep = inspect.signature(hil_session_guards._fast_fade_prep)
    assert "production_state" in prep.parameters, \
        "the fast-fade prep must wait for the snapshot it relies on"
    gear = _FadingGear(fade_ms=700)
    snap = {"devices": {"10": {"config": prod_state._gear_config(
        gear.attributes(10)["attributes"])}}}
    done, failed = prod_state.fast_fade(gear, [10], log=lambda _: None)
    assert (done, failed, gear.fade_ms) == ([10], [], 0)
    prod_state._restore_gear_config(gear, snap, lambda _: None,
                                    write_log.WriteLog.everything("t", "b"))
    assert gear.writes == [(10, {"fade_time_ms": 0}), (10, {"fade_time_ms": 700})]
    assert gear.fade_ms == 700


def test_fast_fade_without_the_guard_is_a_usage_error(monkeypatch):
    monkeypatch.setenv("HIL_STATE_GUARD", "0")
    config = types.SimpleNamespace(getoption=lambda name: name == "--fast-fade")
    with pytest.raises(pytest.UsageError, match="HIL_STATE_GUARD=0"):
        hil_session._refuse_unguarded_fast_fade(config)
    monkeypatch.setenv("HIL_STATE_GUARD", "1")
    hil_session._refuse_unguarded_fast_fade(config)


POLICY = {"system_failure_level": None, "power_on_level": 254, "apply_on_discovery": False,
          "manages_anything": True}


class _Policies:
    def __init__(self, now):
        self.now, self.patches = dict(now), []

    def _req(self, method, path, body=None):
        assert path == "policies"
        if method == "PATCH":
            self.patches.append(body)
            self.now.update(body)
        return dict(self.now)


def test_a_policy_is_put_back_field_by_field_and_never_its_derived_flag():
    api = _Policies(dict(POLICY, power_on_level=200, apply_on_discovery=True,
                         manages_anything=False))
    prod_state.restore_policies(api, POLICY, log=lambda line: None)
    assert api.patches == [{"power_on_level": 254, "apply_on_discovery": False}]
    assert prod_state.policy_patch(POLICY, api.now) == {}


def test_a_policy_that_holds_is_not_written():
    api = _Policies(POLICY)
    prod_state.restore_policies(api, POLICY, log=lambda line: None)
    assert api.patches == []


def _leaf(value, source, confirmed_at):
    return {"value": value, "source": source, "last_write_confirmed_ms": confirmed_at}


def test_a_level_counts_as_written_only_with_a_fresh_confirmed_write():
    wanted = {"power_on_level": 254}
    before = {4: {"power_on_level": _leaf(254, "write_confirmed", 100)},
              5: {"power_on_level": _leaf(200, "readback", None)}}
    fresh = {4: {"power_on_level": _leaf(254, "write_confirmed", 900)},
             5: {"power_on_level": _leaf(254, "write_confirmed", 910)}}
    assert test_policies.unconfirmed_levels(before, fresh, wanted) == []
    stale = {4: {"power_on_level": _leaf(254, "write_confirmed", 100)},
             5: {"power_on_level": _leaf(254, "readback", 910)}}
    assert len(test_policies.unconfirmed_levels(before, stale, wanted)) == 2
    wrong = {4: {"power_on_level": _leaf(200, "write_confirmed", 900)},
             5: {"power_on_level": _leaf(254, "write_confirmed", None)}}
    assert len(test_policies.unconfirmed_levels(before, wrong, wanted)) == 2


OWNER_DOC = 'rule "night" {\n  when at 23:00\n  do broadcast.off()\n}'
RULES_WRITTEN = write_log.WriteLog("t", "http://dut", touched={"rules": {"*"}})
TEST_RULE = http_rule("hil-vg-06-stop-fade", GROUP_TARGET, 4, "stop_fade()")


class _Rules:
    def __init__(self, source, toggles, revision=7, conflict=False, pending=0):
        self.source, self.toggles, self.revision = source, dict(toggles), revision
        self.conflict, self.puts, self.patches = conflict, [], []
        self.pending = pending

    def stats(self):
        gauge = {} if self.pending is None else {"continuations_pending": self.pending}
        return {"rules": dict(gauge, continuations_dropped=0)}

    def rules_get(self):
        return {"source": self.source, "revision": self.revision, "diagnostic": None}

    def rules_toggles(self):
        return dict(self.toggles)

    def rules_replace(self, source, base):
        self.puts.append((source, base))
        if self.conflict or base != self.revision:
            raise api_mod.ApiError(409, {"error": "rule_set_conflict"}, "rules")
        self.source, self.revision = source, self.revision + 1
        self.toggles = {name: True for name in self.toggles}
        return {"status": "succeeded"}

    def rule_enable(self, name, enabled):
        self.patches.append((name, enabled))
        self.toggles[name] = enabled
        self.revision += 1

    def _req(self, method, path, body=None):
        names = re.findall(r'rule "([^"]+)"', self.source)
        return {"rules": {"rules": [{"name": n, "enabled": self.toggles.get(n, True)}
                                    for n in names]}}


def _snap_rules(source, toggles):
    return {"rules": {"source": source}, "rule_toggles": dict(toggles)}


def test_the_session_takes_its_test_rules_out_and_puts_back_the_toggles_its_commit_reset():
    api = _Rules(OWNER_DOC + "\n\n" + TEST_RULE, {"night": False, "hil-vg-06-stop-fade": True})
    prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": False}), lambda line: None,
                              RULES_WRITTEN)
    assert api.puts == [(OWNER_DOC, 7)] and api.patches == [("night", False)]
    moved = _Rules(OWNER_DOC + "\n\n" + TEST_RULE, {"night": True, "hil-vg-06-stop-fade": True})
    prod_state._restore_rules(moved, _snap_rules(OWNER_DOC, {"night": False}), lambda line: None,
                              RULES_WRITTEN)
    assert moved.puts == [(OWNER_DOC, 7)] and moved.patches == [] and moved.toggles["night"]


OWNER_NEW = 'rule "porch" {\n  when at 06:00\n  do group(9).on()\n}'


def test_an_owner_rule_written_after_a_test_rule_keeps_the_document_as_it_is():
    for joint in ("\n", "\n\n"):
        source = OWNER_DOC + "\n\n" + TEST_RULE + joint + OWNER_NEW
        api, log = _Rules(source, {"night": True}), []
        prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": True}), log.append,
                                  RULES_WRITTEN)
        assert api.puts == [] and api.source == source
        assert any("someone else edited it" in line for line in log)


def test_the_session_leaves_a_toggle_it_did_not_move_and_reports_it():
    api = _Rules(OWNER_DOC, {"night": True})
    prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": False}), lambda line: None,
                              RULES_WRITTEN)
    assert api.puts == [] and api.patches == []
    assert "rule 'night' enabled False -> True" in prod_state.diff(
        {"rules": {"source": OWNER_DOC}, "rule_toggles": {"night": False}, **_bare()},
        {"rules": {"source": OWNER_DOC}, "rule_toggles": {"night": True}, **_bare()})


def test_an_owner_edit_is_left_in_place_and_reported():
    edited = OWNER_DOC.replace("23:00", "22:00")
    api, log = _Rules(edited, {"night": True}), []
    prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": True}), log.append,
                                  RULES_WRITTEN)
    assert api.puts == [] and any("someone else edited it" in line for line in log)
    assert prod_state.diff(
        {"rules": {"source": OWNER_DOC}, "rule_toggles": {"night": False}, **_bare()},
        {"rules": {"source": edited}, "rule_toggles": {"night": True}, **_bare()}) == [
        "rules source differs", "rule 'night' enabled False -> True"]


def _bare():
    snap = _snap()
    return {k: v for k, v in snap.items() if k not in ("rules",)}


def test_a_gear_config_the_guard_refuses_is_left_and_the_rest_restored(monkeypatch):
    class _Gear:
        def __init__(self):
            self.written = []

        def attributes(self, short):
            return {"attributes": {"common_102": {"fade_time_ms": {"value": 700}}}}

        def write_attrs(self, short, body):
            if short == 6:
                raise LampNotAllowed("attribute write to SA6 refused")
            self.written.append(short)
            return {"operation_id": "op"}

        def wait_op(self, op):
            return {"status": "succeeded"}

    gear, log = _Gear(), []
    snap = {"devices": {"6": {"config": {"fade_time_ms": 0}},
                        "7": {"config": {"fade_time_ms": 0}}}}
    prod_state._restore_gear_config(gear, snap, log.append,
                                    write_log.WriteLog.everything("t", "b"))
    assert gear.written == [7] and any("SA6 gear config left" in line for line in log)


def test_a_rules_residue_names_the_document_and_every_toggle_that_moved():
    toggles = {"night": False, "day": True}
    assert hil_test_guards.rules_residue("a", "a", toggles, dict(toggles)) == []
    residue = hil_test_guards.rules_residue("a", "a\n\nb", toggles,
                                            {"night": True, "day": True, "hil-x": True})
    assert residue == ["the document is not the one the test found",
                       "rule 'hil-x' is enabled, was absent",
                       "rule 'night' is enabled, was disabled"]



def test_pending_delays_are_read_from_the_firmware_gauge_and_never_estimated():
    assert prod_state.continuations_pending(_Rules(OWNER_DOC, {}, pending=3)) == 3
    assert prod_state.continuations_pending(_Rules(OWNER_DOC, {}, pending=None)) is None
    assert prod_state.commit_refusal(0) is None
    assert "3 delayed action(s)" in prod_state.commit_refusal(3)
    assert "reports no rules.continuations_pending" in prod_state.commit_refusal(None)


def test_a_live_owner_document_refuses_a_commit():
    with pytest.raises(pytest.skip.Exception, match="switched off"):
        hil_test_guards._refuse_a_live_owner_document({"night": False}, 0)
    with pytest.raises(pytest.skip.Exception, match="1 delayed action"):
        hil_test_guards._refuse_a_live_owner_document({"night": True}, 1)
    with pytest.raises(pytest.skip.Exception, match="reports no rules.continuations_pending"):
        hil_test_guards._refuse_a_live_owner_document({"night": True}, None)
    hil_test_guards._refuse_a_live_owner_document({"night": True}, 0)


def test_the_guard_checks_the_owner_document_again_right_before_each_commit():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    api.pending = 2
    with pytest.raises(pytest.skip.Exception, match="2 delayed action"):
        commits.append(TEST_RULE)
    api.pending, api.toggles["night"] = 0, False
    with pytest.raises(pytest.skip.Exception, match="switched off"):
        commits.append(TEST_RULE)
    assert api.puts == [] and commits.restore() == []


def test_the_guard_keeps_a_toggle_the_owner_moved_during_the_test_and_reports_it():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    commits.append(TEST_RULE)
    api.toggles["night"] = False
    residue = commits.restore()
    assert api.source == OWNER_DOC and api.toggles["night"] is False
    assert residue == ["rule 'night' is disabled, was enabled"]


def test_a_delay_pending_at_restore_keeps_the_test_rule_and_names_why():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    commits.append(TEST_RULE)
    api.pending = 1
    residue = commits.restore()
    assert len(api.puts) == 1 and api.source.endswith(TEST_RULE)
    assert api.patches == [("hil-vg-06-stop-fade", False)]
    assert residue and "switched off" in residue[0] and "1 delayed action(s)" in residue[0]


def test_a_commit_that_fails_after_its_replace_still_leaves_the_test_rule_to_restore():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    replace, enable = api.rules_replace, api.rule_enable

    def replace_and_reset(source, base):
        view = replace(source, base)
        api.toggles["night"] = False
        return view

    def overloaded(name, enabled):
        raise api_mod.ApiError(503, {"error": "commands_ingress_overload"}, "rules")

    api.rules_replace, api.rule_enable = replace_and_reset, overloaded
    with pytest.raises(api_mod.ApiError):
        commits.append(TEST_RULE)
    assert commits.ours == 8
    api.rules_replace, api.rule_enable = replace, enable
    assert commits.restore() == [] and api.source == OWNER_DOC


def test_the_session_restore_leaves_test_rules_while_the_owner_has_delays_pending():
    for pending, why in ((1, "1 delayed action(s)"), (None, "reports no rules.")):
        api, log = _Rules(OWNER_DOC + "\n\n" + TEST_RULE, {"night": True}, pending=pending), []
        prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": True}), log.append,
                                  RULES_WRITTEN)
        assert api.puts == [] and any("stay in the rules document" in line and why in line
                                      and "switched off" in line for line in log)
        assert api.patches == [("hil-vg-06-stop-fade", False)]


def test_the_guard_restores_against_the_revision_its_own_commit_left():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    commits.append(TEST_RULE)
    assert api.puts == [(OWNER_DOC + "\n\n" + TEST_RULE, 7)] and commits.ours == 8
    assert commits.restore() == []
    assert api.puts[-1] == (OWNER_DOC, 8) and api.source == OWNER_DOC


def test_an_owner_edit_during_the_test_is_a_named_failure_not_an_overwrite():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    commits.append(TEST_RULE)
    api.source, api.revision = api.source + "\n# owner", api.revision + 1
    with pytest.raises(pytest.fail.Exception, match="changed while the test held it"):
        commits.restore()
    assert api.source.endswith("# owner")
    assert api.patches == [("hil-vg-06-stop-fade", False)]


def test_a_leftover_test_rule_is_switched_off_after_the_owner_edited_the_document():
    edited = OWNER_DOC.replace("23:00", "22:00")
    api, log = _Rules(edited + "\n\n" + TEST_RULE, {"night": True}), []
    prod_state._restore_rules(api, _snap_rules(OWNER_DOC, {"night": True}), log.append,
                              RULES_WRITTEN)
    assert api.puts == [] and api.patches == [("hil-vg-06-stop-fade", False)]
    assert any("switched off" in line for line in log)


def test_a_test_that_never_committed_does_not_judge_the_owner_toggles():
    api = _Rules(OWNER_DOC, {"night": True})
    commits = hil_test_guards._RulesCommits(api, api.rules_get(), {"night": True})
    api.toggles["night"] = False
    assert commits.restore() == [] and api.puts == []


def test_an_unread_attribute_the_test_would_write_refuses_the_gear():
    with pytest.raises(pytest.skip.Exception, match="SA4 did not report power_on_level"):
        hil_test_guards.refuse_unread(4, ("power_on_level", "system_failure_level"),
                                      {"power_on_level": None, "system_failure_level": 254},
                                      None)
    hil_test_guards.refuse_unread(4, ("power_on_level",), {"power_on_level": None}, 254)


TIMED = {"rules": {"rules": [
    {"name": "dusk", "enabled": True, "triggers": [{"kind": "at_solar", "event": "sunset",
                                                    "offset_ms": 0}]},
    {"name": "night", "enabled": False, "triggers": [{"kind": "at_time",
                                                      "time": {"hour": 23, "minute": 0}}]},
    {"name": "hall", "enabled": True, "triggers": [{"kind": "http_trigger"}]},
]}}


class _ClockApi:
    def __init__(self, compiled, read_only=False):
        self.compiled, self.calls = compiled, []
        self.cfg = types.SimpleNamespace(lamps_read_only=read_only)

    def _req(self, method, path, body=None):
        return self.compiled

    def time_get(self):
        return {"timezone": "MSK-3"}

    def time_set(self, **kwargs):
        self.calls.append(kwargs)


def test_timed_rules_are_the_enabled_ones_a_time_or_the_sun_fires():
    assert api_mod.timed_rules_of(TIMED) == ["dusk"]
    assert api_mod.timed_rules_of({"rules": None}) == []


def test_the_clock_moves_on_the_first_call_and_never_under_an_owner_timed_rule():
    quiet = _ClockApi({"rules": {"rules": TIMED["rules"]["rules"][1:]}})
    clock = hil_test_guards._Clock(quiet)
    assert quiet.calls == []
    clock.at(10, 0)
    assert quiet.calls[0] == {"timezone": hil_test_guards.ANCHOR_TZ} and len(quiet.calls) == 2
    clock.restore()
    assert quiet.calls[-1] == {"timezone": "MSK-3"}
    timed = _ClockApi(TIMED)
    clock = hil_test_guards._Clock(timed)
    with pytest.raises(pytest.skip.Exception, match=r"\['dusk'\] fire at a time of day"):
        clock.at(10, 0)
    clock.restore()
    assert timed.calls == []
    read_only = _ClockApi({"rules": {"rules": []}}, read_only=True)
    with pytest.raises(pytest.skip.Exception, match=r"read-only run never moves"):
        hil_test_guards._Clock(read_only).at(10, 0)
    assert read_only.calls == []


class _Overrides:
    adapter = 0

    def __init__(self, allowed=()):
        self.cfg = types.SimpleNamespace(lamp_short_set=lambda: frozenset(allowed))
        self.records = {1: None, 2: None}
        self.patches = []

    def addrs(self):
        return sorted(self.records)

    def state(self, short):
        return {"supported_device_types": [6], "device_type_override": self.records[short]}

    def device_patch(self, short, body):
        self.patches.append((short, body))
        if body["device_type_override"] == "dt8_color":
            raise api_mod.ApiError(422, {"error": "unsupported_device_type"}, "devices")
        self.records[short] = body["device_type_override"]


def test_the_override_probe_writes_nothing_to_a_lamp_the_run_may_not_write():
    owner = _Overrides()
    with pytest.raises(pytest.skip.Exception, match=r"no lamp of HIL_LAMP_SHORTS"):
        test_attributes.test_a_widening_device_type_override_is_refused(owner, None)
    assert owner.patches == []
    allowed = _Overrides(allowed={2})
    test_attributes.test_a_widening_device_type_override_is_refused(allowed, None)
    assert allowed.patches == [(2, {"device_type_override": "dt8_color"}),
                               (2, {"device_type_override": "unknown"}),
                               (2, {"device_type_override": None})]


def _writes(touched):
    return write_log.WriteLog("t", "http://dut", touched=touched)


class _Knob:
    def __init__(self, state):
        self.state, self.patches = dict(state), []

    def get(self):
        return dict(self.state)

    def patch(self, body):
        self.patches.append(body)
        self.state.update(body)


class _SettingsApi:
    def __init__(self, poller, dali):
        self.poller, self.dali_settings = _Knob(poller), _Knob(dali)
        self.ha = _Knob({})
        self.redundancy = types.SimpleNamespace(settings=lambda: {},
                                                patch_settings=lambda body: None)


def test_the_session_restore_writes_back_only_the_settings_the_toolkit_wrote():
    api, lines = _SettingsApi({"enabled": True}, {"application_active": False}), []
    snap = {"settings": {"poller": {"enabled": False}, "dali": {"application_active": True},
                         "redundancy": {}, "ha": {}}}
    prod_state._restore_settings(api, snap, lines.append,
                                 _writes({"settings/poller": {"enabled"}}))
    assert api.poller.patches == [{"enabled": False}] and api.dali_settings.patches == []
    assert any("dali application_active" in line and "did not write" in line for line in lines)


class _DeviceApi:
    def __init__(self, record):
        self.record, self.patches = dict(record), []

    def state(self, short):
        return dict(self.record)

    def device_patch(self, short, body):
        self.patches.append((short, body))


def test_the_session_restore_leaves_a_device_field_the_owner_changed():
    api, lines = _DeviceApi({"name": "b", "notes": "owner's"}), []
    snap = {"devices": {"5": {"record": {"name": "a", "notes": "n"}}}}
    prod_state._restore_devices(api, snap, lines.append, _writes({"device/5": {"name"}}))
    assert api.patches == [(5, {"name": "a"})]
    assert any("SA5 notes" in line and "did not write" in line for line in lines)


class _HclApi:
    def __init__(self, schedules):
        self.schedules, self.calls = {s["schedule_id"]: s for s in schedules}, []
        self.hcl = self

    def list(self):
        return list(self.schedules.values())

    def delete(self, sid):
        self.calls.append(("delete", sid))

    def create(self, body):
        self.calls.append(("create", body["schedule_id"]))

    def patch(self, sid, body):
        self.calls.append(("patch", sid, body))


def test_the_session_restore_undoes_only_the_schedules_the_toolkit_wrote():
    api, lines = _HclApi([{"schedule_id": "owner", "enabled": False, "points": [1]},
                          {"schedule_id": "hil-x", "enabled": True},
                          {"schedule_id": "owner-new", "enabled": True}]), []
    snap = {"hcl": [{"schedule_id": "owner", "enabled": True, "points": [2]}]}
    prod_state._restore_hcl(api, snap, lines.append,
                            _writes({"hcl/owner": {"enabled"}, "hcl/hil-x": {"*"}}))
    assert sorted(api.calls, key=str) == [("delete", "hil-x"),
                                          ("patch", "owner", {"enabled": True})]
    assert any("owner-new" in line for line in lines)
    assert any("owner points" in line and "did not write" in line for line in lines)


class _GearApi:
    def __init__(self, attrs):
        self.attrs, self.written = attrs, []

    def attributes(self, short, sections=None):
        return {"attributes": {"common_102": {k: {"value": v} for k, v in self.attrs.items()}}}

    def write_attrs(self, short, body):
        self.written.append((short, body))
        return {"operation_id": "op"}

    def wait_op(self, op):
        return {"status": "succeeded"}


def test_the_session_restore_writes_only_the_gear_fields_the_toolkit_wrote():
    api, lines = _GearApi({"fade_time_ms": 700, "max_level": 200}), []
    snap = {"devices": {"5": {"config": {"fade_time_ms": 0, "max_level": 254}}}}
    prod_state._restore_gear_config(api, snap, lines.append,
                                    _writes({"gear/5": {"fade_time_ms"}}))
    assert api.written == [(5, {"fade_time_ms": 0})]
    assert any("max_level" in line and "did not write" in line for line in lines)


class _ZoneApi:
    def __init__(self):
        self.sets = []

    def time_get(self):
        return {"timezone": "UTC0"}

    def time_set(self, timezone):
        self.sets.append(timezone)


def test_the_session_restore_moves_the_zone_back_only_if_the_toolkit_moved_it():
    api, lines = _ZoneApi(), []
    prod_state._restore_timezone(api, {"timezone": "MSK-3"}, lines.append, _writes({}))
    assert api.sets == [] and any("did not write" in line for line in lines)
    prod_state._restore_timezone(api, {"timezone": "MSK-3"}, lines.append,
                                 _writes({"time": {"timezone"}}))
    assert api.sets == ["MSK-3"]


def test_without_a_write_log_the_restore_writes_nothing_and_names_the_full_restore(monkeypatch):
    def refuse(*args, **kwargs):
        raise AssertionError("a restore step ran without a write log")

    for name in ("_restore_settings", "_restore_devices", "_restore_hcl",
                 "_restore_gear_config", "_restore_rules"):
        monkeypatch.setattr(prod_state, name, refuse)
    monkeypatch.setattr(prod_state, "capture", lambda api, prime=True, log=print: {})
    monkeypatch.setattr(prod_state, "diff", lambda before, after, shown_shorts=None:
                        ["settings/poller.enabled False -> True"])
    lines = []
    residual = prod_state.restore(object(), {}, None, log=lines.append)
    assert residual == ["settings/poller.enabled False -> True"]
    assert any("hil state restore --all" in line for line in lines)


class _Overridden:
    def __init__(self):
        self.cleared = []
        self.hcl = self

    def clear_override(self, sid):
        self.cleared.append(sid)


def test_an_hcl_override_is_cleared_only_when_the_toolkit_drove_a_lamp_it_targets():
    snap = {"hcl_overrides": {"evening": False},
            "hcl": [{"schedule_id": "evening", "targets": [{"scope": "group",
                                                             "group_ids": [3]}]}],
            "vl": {"virtual_lamps": [{"virtual_lamp_id": 7,
                                      "binding": {"physical_short_address": 5}}]},
            "group_matrix": {"rows": [{"virtual_lamp_id": 7,
                                       "applied": [False] * 3 + [True] + [False] * 12}]}}
    after = {"hcl_overrides": {"evening": True}}
    for touched, cleared in (({"shown/*": {"*"}}, []), ({"shown/5": {"*"}}, ["evening"]),
                             ({"hcl_override/evening": {"*"}}, ["evening"])):
        api, lines = _Overridden(), []
        prod_state._clear_session_overrides(api, snap, after, lines.append, _writes(touched))
        assert api.cleared == cleared

