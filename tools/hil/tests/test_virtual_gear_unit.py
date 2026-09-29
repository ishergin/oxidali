import json
import subprocess

import pytest

from hil import tripwire, virtual_gear, wait
from hil.lamp_guard import (GROUP_TARGET, LampGuard, LampNotAllowed, RulesBaseline, VirtualFence,
                            appended_test_rules, http_rule, spell)

WB_CONF = {"gateways": [{"device_id": "wb-dali_19", "buses": [
    {"devices": [{"short": 0}, {"short": 1}, {"short": 12}, {"short": 0, "dali2": True}]},
    {"devices": [{"short": 30}]}]}]}

OWNER_RULES = ('rule "подсветка: включить" {\n  when group(0) becomes any_on\n'
               '  do lamp("Подсветка").on()\n}\nrule "кнопка 3" {\n  when input(dev=0, inst=2)'
               ' is short_press\n}\n')

GROUP_4 = [False] * 4 + [True] + [False] * 11
GROUP_0 = [True] + [False] * 15


def test_the_reserve_covers_the_floor_and_every_known_address():
    reserved = virtual_gear.reserve({1, 4, 12}, {0, 13}, {20})
    assert set(range(16)) <= reserved and {20} <= reserved
    assert spell(reserved) == "0-15,20"


def test_the_park_takes_the_first_free_addresses():
    assert virtual_gear.park_shorts(frozenset(range(16)) | {17}, 3) == [16, 18, 19]
    with pytest.raises(virtual_gear.VirtualGearError):
        virtual_gear.park_shorts(frozenset(range(62)), 3)


def test_a_park_shape_names_three_counts():
    assert virtual_gear.parse_park(" 6, 5,5 ") == (6, 5, 5)
    assert virtual_gear.parse_park("0,1,0") == (0, 1, 0)
    for spec in ("6,5", "6,5,5,1", "6,-5,5", "a,b,c", "0,0,0", ""):
        with pytest.raises(virtual_gear.VirtualGearError):
            virtual_gear.parse_park(spec)


def test_a_kind_is_the_slice_the_fleet_builds_for_it():
    park = list(range(16, 32))
    assert virtual_gear.park_of_kind(park, (6, 5, 5), "dt6") == list(range(16, 22))
    assert virtual_gear.park_of_kind(park, (6, 5, 5), "cct") == list(range(22, 27))
    assert virtual_gear.park_of_kind(park, (6, 5, 5), "rgb") == list(range(27, 32))
    assert virtual_gear.park_of_kind(park[:8], (8, 0, 0), "cct") == []


def test_the_wb_list_is_read_for_the_named_gateway_and_bus_without_part_103_units():
    assert virtual_gear.wb_shorts_of(WB_CONF, "wb-dali_19", 1) == {0, 1, 12}
    assert virtual_gear.wb_shorts_of(WB_CONF, "wb-dali_19", 2) == {30}
    assert virtual_gear.wb_shorts_of(WB_CONF, "other", 1) == set()


def _wb_cfg():
    return type("Cfg", (), {"wb_ssh": "root@wb", "wb_device": "wb-dali_19", "wb_bus": 1})()


def test_an_empty_wb_list_refuses_the_session(monkeypatch):
    monkeypatch.setattr(virtual_gear.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(
        a, 0, stdout=json.dumps({"gateways": []}), stderr=""))
    with pytest.raises(virtual_gear.VirtualGearError, match="HIL_WB_DEVICE"):
        virtual_gear.wb_shorts(_wb_cfg())


def test_free_groups_leave_out_applied_and_desired_ones():
    devices = [{"groups_membership": 0b0001}, {"groups_membership": 0b0100}]
    rows = [{"desired": [False, True] + [False] * 14,
             "applied": [False, False, False, True] + [False] * 12}]
    assert virtual_gear.free_groups(virtual_gear.used_groups(devices, rows)) == \
        list(range(4, 16))


def test_an_unread_membership_frees_no_group():
    devices = [{"groups_membership": 1}, {"groups_membership": None}]
    assert virtual_gear.free_groups(virtual_gear.used_groups(devices, [])) == []


class _WireApi:
    def __init__(self, answering, contended=0):
        self.answering, self.sent, self.contended = set(answering), [], contended
        self.frames = 0

    def cmd_wire(self, addr, opcode):
        self.sent.append((addr, opcode))
        if self.contended:
            self.contended -= 1
            self.frames += 1
        group = (addr >> 1) & 0x0F
        return {"success": group in self.answering, "backward_frame": 0xFF
                if group in self.answering else 0}

    def stats(self):
        return {"dali": {"collision_restarts_total": 0, "foreign_frames_total": self.frames}}


def test_free_groups_are_proven_silent_after_a_positive_control():
    api = _WireApi({0, 2})
    assert virtual_gear.prove_groups_empty(api, [4, 5], {0, 2}) == 0
    assert api.sent.count((0x89, 0x91)) == virtual_gear.PROOF_PROBES
    assert api.sent.count((0x81, 0x91)) == virtual_gear.PROOF_PROBES


def test_no_free_group_needs_no_proof():
    api = _WireApi({0})
    assert virtual_gear.prove_groups_empty(api, [], None) is None and api.sent == []


def test_silence_without_a_positive_control_proves_nothing():
    with pytest.raises(virtual_gear.VirtualGearError, match="prove nothing"):
        virtual_gear.prove_groups_empty(_WireApi(set()), [4], {0})


def test_a_free_group_that_answers_is_occupied():
    with pytest.raises(virtual_gear.VirtualGearError, match=r"\[5\]"):
        virtual_gear.prove_groups_empty(_WireApi({0, 5}), [4, 5], {0})


def test_a_proof_on_a_shared_wire_is_repeated():
    api = _WireApi({0}, contended=1)
    assert virtual_gear.prove_groups_empty(api, [4], {0}) == 0
    assert api.sent.count((0x89, 0x91)) == 2 * virtual_gear.PROOF_PROBES


def test_a_wire_contended_on_every_attempt_proves_nothing():
    api = _WireApi({0}, contended=100)
    with pytest.raises(virtual_gear.VirtualGearError, match="shared the wire"):
        virtual_gear.prove_groups_empty(api, [4], {0})


def test_a_violating_answer_counts_as_present():
    assert virtual_gear.answered_yes({"success": False, "backward_violation": True})
    assert not virtual_gear.answered_yes({"success": False, "backward_frame": 0})


def test_only_a_clean_answer_passes_the_first_rung():
    assert virtual_gear.answered_cleanly({"success": True, "backward_frame": 0})
    assert not virtual_gear.answered_cleanly({"success": True, "backward_violation": True})
    assert not virtual_gear.answered_cleanly({"success": False})


COMPILED = {"rules": [
    {"name": "a", "triggers": [{"kind": "group_aggregate", "group": {"adapter_id": 0, "id": 5}},
                               {"kind": "device_online", "device":
                                {"adapter_id": 0, "short_address": 17}}],
     "conditions": [{"kind": "lamp_level", "lamp": {"adapter_id": 0, "id": 61}}],
     "actions": [{"kind": "light_off", "scope": "virtual_lamp", "adapter_id": 0, "id": 60},
                 {"kind": "scene_recall", "target": {"scope": "group", "adapter_id": 0,
                                                     "id": 6}},
                 {"kind": "light_on", "scope": "group", "adapter_id": 1, "id": 4},
                 {"kind": "input_feedback", "device": {"adapter_id": 0,
                                                       "device_short_address": 16}}]}],
    "blocks": []}


def _scope(groups=(4, 5), lamps=(60, 61), park=(16, 17)):
    return virtual_gear.SessionScope.of(groups, lamps, park)


def test_the_compiled_rules_name_every_group_lamp_and_device_they_reach():
    assert virtual_gear.rule_references(COMPILED, 0) == {
        "group": {5, 6}, "lamp": {60, 61}, "device": {17}}
    assert virtual_gear.rule_references(None, 0) == {"group": set(), "lamp": set(),
                                                     "device": set()}


def test_the_owner_rules_may_not_reach_the_session_in_text_or_compiled():
    assert virtual_gear.rules_conflicts(OWNER_RULES, None, _scope(groups=(0, 4)), {}, 0) == \
        ["the owner's rules use group 0"]
    assert virtual_gear.rules_conflicts(OWNER_RULES, None, _scope(groups=(4,)), {}, 0) == []
    assert virtual_gear.rules_conflicts(
        'rule "x" {\n  when device(16) goes offline\n  do lamp(60).off()\n}', None,
        _scope(), {}, 0) == ["the owner's rules use lamp 60",
                             "the owner's rules watch device 16"]
    assert virtual_gear.rules_conflicts(
        'rule "x" {\n  when group("зал") becomes any_on\n  do lamp("virtual gear SA17", '
        'adapter=0).on()\n}', None, _scope(), {"зал": 5}, 0) == [
        "the owner's rules use group 5", "the owner's rules name lamp 'virtual gear SA17'"]
    assert virtual_gear.rules_conflicts("", COMPILED, _scope(), {}, 0) == [
        "the owner's rules use group 5", "the owner's rules use lamp 60",
        "the owner's rules use lamp 61", "the owner's rules watch device 17"]


def test_an_input_device_is_not_a_park_device():
    source = 'rule "x" {\n  when input device(dev=16) power cycled\n  do log("x")\n}'
    assert virtual_gear.rules_conflicts(source, None, _scope(), {}, 0) == []


def test_session_vls_take_ids_nobody_holds():
    assert virtual_gear.free_vl_ids([0, 1, 63], 2) == [61, 62]


def test_retained_residue_ignores_state_and_catches_new_and_changed_config():
    before = {"homeassistant/light/a/config": "A", "dali/lamp/1/state": "on"}
    now = {"homeassistant/light/a/config": "A2", "dali/lamp/1/state": "off",
           "homeassistant/light/b/config": "B"}
    assert virtual_gear.retained_residue(before, now, "homeassistant") == [
        "retained MQTT topic appeared: homeassistant/light/b/config",
        "retained MQTT config changed: homeassistant/light/a/config"]


def _fence(**kw):
    args = dict(park=[16, 17], groups=[4, 5], session_vls=[60, 61])
    args.update(kw)
    return VirtualFence(**args)


def _guard(fence):
    guard = LampGuard({16, 17}, segment=lambda: [1, 4, 16, 17])
    guard.fence = fence
    return guard


@pytest.mark.parametrize("method,path,body", [
    ("PUT", "adapters/0/virtual-lamps/60/target-state", {"power": "on"}),
    ("PUT", "adapters/0/physical-devices/16/target-state", {"level": 100}),
    ("PUT", "adapters/0/groups/4/target-state", {"power": "on"}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 60, "desired": GROUP_4}]}),
    ("PATCH", "adapters/0/scenes/15/matrix",
     {"rows": [{"virtual_lamp_id": 60, "desired": {"included": True, "level": 90}}]}),
    ("POST", "adapters/0/commissioning/identify", {"short_address": 16}),
    ("POST", "dali/command", {"wire_address": 0x0D, "command": 0x91}),
    ("POST", "adapters/0/physical-devices/16/attribute-reads",
     {"attribute_groups": ["dt8_color"], "memory_banks": "none"}),
    ("POST", "adapters/0/physical-devices/17/write-attributes", {"fade_time_ms": 1000}),
])
def test_the_fence_lets_the_session_reach_its_own_gear(method, path, body):
    _guard(_fence()).check_request(method, path, body)


@pytest.mark.parametrize("method,path,body", [
    ("PUT", "adapters/0/groups/0/target-state", {"power": "on"}),
    ("PUT", "adapters/0/virtual-lamps/6/target-state", {"power": "on"}),
    ("PUT", "adapters/0/physical-devices/6/target-state", {"level": 100}),
    ("POST", "adapters/0/scenes/3/recall", {"scope": "group", "group_id": 5}),
    ("POST", "hcl-schedules", {"targets": [{"scope": "group", "group_ids": [4]}]}),
    ("PATCH", "hcl-schedules/moscow-cct", {"enabled": True}),
    ("DELETE", "hcl-schedules/moscow-cct", None),
    ("PUT", "adapters/0/group-membership-matrix", {"rows": []}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 6, "desired": GROUP_4}]}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 60, "desired": GROUP_0}]}),
    ("PATCH", "adapters/0/groups/4", {"ha_entity_enabled": True}),
    ("PATCH", "adapters/0/scenes/2", {"name": "x"}),
    ("PATCH", "adapters/0/virtual-lamps/60", {"ha_entity_enabled": True}),
    ("PUT", "adapters/0/virtual-lamps/60/binding", {"physical_short_address": 16}),
    ("DELETE", "adapters/0/virtual-lamps/6", None),
    ("DELETE", "adapters/0/physical-devices/16", None),
    ("POST", "adapters/0/physical-devices/6/write-attributes", {"fade_time_ms": 0}),
    ("POST", "adapters/0/physical-devices/6/attribute-reads", {"attribute_groups": ["groups"]}),
    ("PUT", "adapters/0/physical-devices/16/write-attributes", {"fade_time_ms": 0}),
    ("PATCH", "adapters/0/physical-devices/16", {"name": "x"}),
    ("PATCH", "policies", {"apply_on_discovery": True}),
    ("POST", "adapters/0/discovery-runs", {"mode": "scan_known_short_addresses"}),
    ("POST", "adapters/0/commissioning/address-changes", {"short_address": 16}),
    ("POST", "adapters/0/commissioning/replacements", {"short_address": 16}),
    ("POST", "adapters/0/commissioning/identify", {"short_address": 6}),
    ("POST", "adapters/0/commissioning/steps/initialise", {"scope": "unaddressed"}),
    ("PUT", "rules", {"source": 'rule "t" { when group(4) becomes any_on do broadcast.off() }'}),
    ("POST", "rules/t/run", None),
    ("PUT", "time", {"unix_ms": 0}),
    ("PATCH", "settings/home-assistant", {"enabled": True}),
    ("POST", "firmware/updates", {"url": "http://x/y.bin"}),
    ("POST", "redundancy/switchover", None),
])
def test_the_fence_refuses_everything_else(method, path, body):
    with pytest.raises(LampNotAllowed):
        _guard(_fence()).check_request(method, path, body)


OWNER_TOGGLES = {"подсветка: включить": True, "кнопка 3": False}


def _ruled(**kw):
    return _fence(rules=RulesBaseline(OWNER_RULES, dict(OWNER_TOGGLES)), **kw)


def _with(fragment):
    return {"source": OWNER_RULES + "\n\n" + fragment, "base_revision": 3}


def test_the_fence_passes_the_owner_rules_plus_an_http_rule_on_the_session_and_its_run():
    guard = _guard(_ruled())
    guard.check_request("PUT", "rules", _with(http_rule("hil-a", GROUP_TARGET, 4, "stop_fade()")))
    guard.check_request("POST", "rules/hil-a/run", {})
    guard.check_request("POST", "rules/hil-a/run?dry=1", {})
    guard.check_request("PUT", "rules", _with(http_rule("hil-b", "lamp", 61, "level(90)")))
    guard.check_request("PUT", "rules", {"source": OWNER_RULES, "base_revision": 4})


@pytest.mark.parametrize("body", [
    _with(http_rule("hil-a", GROUP_TARGET, 0, "stop_fade()")),
    _with(http_rule("hil-a", "lamp", 6, "on()")),
    _with('rule "hil-a" {\n  when http trigger\n  do   broadcast.off()\n}'),
    _with('rule "hil-a" {\n  when group(4) becomes any_on\n  do   group(4).off()\n}'),
    _with('rule "owner-like" {\n  when http trigger\n  do   group(4).off()\n}'),
    _with(http_rule("hil-a", GROUP_TARGET, 4, "stop_fade()") + "\n"),
    {"source": OWNER_RULES.replace("кнопка 3", "кнопка 4"), "base_revision": 3},
    {"source": "", "base_revision": 3},
    {"base_revision": 3},
])
def test_the_fence_refuses_any_other_rules_document(body):
    with pytest.raises(LampNotAllowed):
        _guard(_ruled()).check_request("PUT", "rules", body)


def test_without_a_baseline_the_fence_refuses_every_rules_write():
    with pytest.raises(LampNotAllowed):
        _guard(_fence()).check_request("PUT", "rules", _with(
            http_rule("hil-a", GROUP_TARGET, 4, "stop_fade()")))


def test_a_run_needs_a_rule_the_fence_passed():
    guard = _guard(_ruled())
    for name in ("hil-a", "%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203"):
        with pytest.raises(LampNotAllowed):
            guard.check_request("POST", "rules/%s/run" % name, {})


@pytest.mark.parametrize("path,body,refused", [
    ("rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203", {"enabled": False}, False),
    ("rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203", {"enabled": True}, True),
    ("rules/hil-a", {"enabled": False}, True),
    ("rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203", {"enabled": False, "x": 1}, True),
])
def test_a_toggle_passes_only_back_to_what_the_owner_had(path, body, refused):
    guard = _guard(_ruled())
    if refused:
        with pytest.raises(LampNotAllowed):
            guard.check_request("PATCH", path, body)
    else:
        guard.check_request("PATCH", path, body)


def test_the_appended_rules_are_read_off_the_owner_document():
    rule = http_rule("hil-a", GROUP_TARGET, 4, "stop_fade()")
    assert appended_test_rules("", rule) == [("hil-a", "group", 4)]
    assert appended_test_rules("x", "x") == []
    two = "x\n\n" + rule + "\n\n" + http_rule("hil-b", "lamp", 60, "on()")
    assert appended_test_rules("x", two) == [("hil-a", "group", 4), ("hil-b", "lamp", 60)]
    assert appended_test_rules("x", "y\n\n" + rule) is None
    assert appended_test_rules("x", "x\n\n") is None


def test_an_apply_is_allowed_only_when_its_diff_is_the_sessions():
    pending = {"rows": {60}}
    fence = _fence(pending=lambda kind, scene: pending["rows"])
    _guard(fence).check_request("POST", "adapters/0/groups/apply", None)
    pending["rows"] = {60, 6}
    with pytest.raises(LampNotAllowed, match="VL6"):
        _guard(fence).check_request("POST", "adapters/0/scenes/1/apply", None)


@pytest.mark.parametrize("step,body,allowed", [
    ("initialise", {"scope": "unaddressed"}, True),
    ("initialise", {}, False),
    ("initialise", {"scope": "all"}, False),
    ("search-address", {"search_address": 0xFFFFFF}, True),
    ("program-short-address", {"short_address": 16}, True),
    ("program-short-address", {"short_address": 6}, False),
    ("verify-short-address", {"short_address": 17}, True),
    ("query-short-address", {}, False),
])
def test_commissioning_steps_stay_on_unaddressed_gear_and_the_park(step, body, allowed):
    guard = _guard(_fence(commissioning=True))
    path = "adapters/0/commissioning/steps/%s" % step
    if allowed:
        guard.check_request("POST", path, body)
    else:
        with pytest.raises(LampNotAllowed):
            guard.check_request("POST", path, body)


@pytest.mark.parametrize("addr,data,commissioning,refused", [
    (0xA5, 0xFF, True, False),
    (0xA5, 0x00, True, True),
    (0xA5, 0xFF, False, True),
    (0xA7, 0x00, False, True),
    (0xB7, (16 << 1) | 1, True, False),
    (0xB7, (6 << 1) | 1, True, True),
    (0xB7, 0xFF, True, True),
    (0xA3, 0x10, False, False),
    (0xC1, 0x08, False, False),
    (0xC7, 0x00, True, True),
    (0xCD, 0x00, True, True),
])
def test_special_frames_are_pinned(addr, data, commissioning, refused):
    guard = _guard(_fence(commissioning=commissioning))
    if refused:
        with pytest.raises(LampNotAllowed):
            guard.check_frame(addr, data)
    else:
        guard.check_frame(addr, data)


@pytest.mark.parametrize("addr,data,refused", [
    (0x89, 0x05, False),
    (0x89, 0x64, False),
    (0x89, 0x60, True),
    (0x81, 0x05, True),
    (0xFE, 0x80, True),
    (0xFF, 0x05, True),
    (0x21, 0x64, False),
    (0x21, 0x60, True),
    (0x21, 0x74, False),
    (0x0D, 0x05, True),
    (0x0C, 0x80, True),
    (0x81, 0x91, False),
    (0x0D, 0x91, False),
])
def test_the_fence_judges_frames(addr, data, refused):
    guard = _guard(_fence())
    if refused:
        with pytest.raises(LampNotAllowed):
            guard.check_frame(addr, data)
    else:
        guard.check_frame(addr, data)


def _tx(frame, batched=False):
    return "I (1) dali: DALI PHY TX: forward16 0x%04x%s (ISR-owned bitbang)" % (
        frame, " (batched)" if batched else "")


def test_the_tripwire_reads_only_frames_actually_sent():
    lines = [_tx(0x2105), _tx(0x2164, batched=True),
             "W (2) x: DALI PHY TX: collision on forward16 0xfe00",
             "I (3) x: DALI PHY TX: forward24 0xfffe12 (ISR-owned bitbang)"]
    assert tripwire.sent_frames(lines) == [(0x21, 0x05), (0x21, 0x64)]


def test_the_tripwire_judges_frames_with_the_fence():
    lines = [_tx(f) for f in (0x2105, 0x8905, 0x8105, 0xfe80, 0x0d05, 0x0d91)]
    lines += [_tx(0xa500, batched=True), _tx(0x2160, batched=True)]
    found = tripwire.violations(lines, _fence(park=[16], groups=[4]))
    assert len(found) == 5
    assert any("group 0" in v for v in found) and any("broadcast" in v for v in found)
    assert any("SA6" in v for v in found) and any("0xA5" in v for v in found)


def test_a_reading_is_taken_once_it_stops_growing():
    grown = [[1], [1, 2], [1, 2, 3]]

    def read():
        return grown.pop(0) if len(grown) > 1 else grown[0]
    assert wait.settled(read, 0.02, 1.0, 0.005) == [1, 2, 3]
    endless = iter(range(1, 10 ** 6))
    assert len(wait.settled(lambda: [0] * next(endless), 0.05, 0.1, 0.001)) > 1


class _Lines:
    def __init__(self, lines):
        self._lines = lines

    def lines(self):
        return self._lines


def test_the_transmit_log_is_read_once_it_falls_quiet():
    window = _Lines([_tx(0x2105), "I (2) x: unrelated", _tx(0x8805, batched=True)])
    assert tripwire.settled_frames(window, 0.01, 0.5, 0.005) == [(0x21, 0x05), (0x88, 0x05)]


@pytest.mark.parametrize("frames,found", [
    ((0xC106, 0x0DE3), 1),
    ((0xC108, 0x0DF2), 1),
    ((0xC108, 0x0DE8), 1),
    ((0xC108, 0x0DF6), 1),
    ((0xC107, 0x0DE0), 1),
    ((0xC108, 0x0DFA), 0),
    ((0xC106, 0x0DED), 0),
    ((0xC107, 0x0DFF), 0),
    ((0x0DE3,), 0),
    ((0xC106, 0xA300, 0x0DE3), 0),
    ((0xC108, 0x21F2), 0),
    ((0xC108, 0x21E2), 0),
])
def test_the_tripwire_reads_an_extended_command_by_the_enable_before_it(frames, found):
    lines = [_tx(frame) for frame in frames]
    assert len(tripwire.violations(lines, _fence(park=[16], groups=[4]))) == found


def test_the_barrier_is_counted_not_merely_seen():
    lines = [_tx(0x2190), _tx(0x2105), _tx(0x2190)]
    assert tripwire.barrier(16) == (0x21, 0x90)
    assert tripwire.barrier_count(lines, 16) == 2


def test_lost_log_lines_are_reported():
    before = {"console_log_dropped_total": 3, "console_log_busy_total": 0}
    after = {"console_log_dropped_total": 5, "console_log_busy_total": 0}
    assert tripwire.lost_lines(before, after) == {"console_log_dropped_total": 2}


class _TeardownApi:
    def __init__(self, failing=()):
        self.calls, self.failing = [], set(failing)
        self.vlamps = self
        self.groups = self

    def _call(self, entry):
        self.calls.append(entry)
        if entry[0] in self.failing:
            raise RuntimeError("%s failed on the bench" % entry[0])

    def delete(self, lamp_id):
        self._call(("vl-delete", lamp_id))

    def device_forget(self, short):
        self._call(("forget", short))

    def patch(self, group, body):
        self._call(("group", group, body))

    def _req(self, method, path, body=None):
        self._call((method, path, body))
        return {}


class _QuietSim:
    def __init__(self):
        self.disabled = []

    def disable(self, which):
        self.disabled.append(which)


def _session(tmp_path, api, sim, **ledger):
    cfg = type("Cfg", (), {"state_dir": tmp_path})()
    session = virtual_gear.VirtualSession(cfg, api, sim)
    session.ledger.update(**ledger)
    return session


def test_teardown_deletes_vls_before_devices_and_restores_flags_and_policy(tmp_path, monkeypatch):
    api, sim = _TeardownApi(), _QuietSim()
    session = _session(tmp_path, api, sim, park=[16, 17], created_vls=[60, 61],
                       group_flags={"4": True}, policy_rearm=True)
    monkeypatch.setattr(session, "residual", lambda: [])
    assert session.close() == []
    assert sim.disabled == ["all"]
    assert api.calls == [("vl-delete", 60), ("vl-delete", 61), ("forget", 16), ("forget", 17),
                         ("group", 4, {"ha_entity_enabled": True}),
                         ("PATCH", "policies", {"apply_on_discovery": True})]
    assert not session.ledger.exists()


def test_a_failing_step_leaves_residue_and_the_rest_still_runs(tmp_path, monkeypatch):
    api = _TeardownApi(failing={"vl-delete"})
    session = _session(tmp_path, api, None, park=[16], created_vls=[60],
                       group_flags={"4": None}, policy_rearm=True)
    monkeypatch.setattr(session, "residual", lambda: [])
    residue = session.close()
    assert ("forget", 16) in api.calls
    assert ("PATCH", "policies", {"apply_on_discovery": True}) in api.calls
    assert any("VL60" in line for line in residue)
    assert any("group 4" in line for line in residue)
    assert session.ledger.exists()


def test_a_teardown_with_residue_keeps_its_ledger(tmp_path, monkeypatch):
    session = _session(tmp_path, _TeardownApi(), _QuietSim(), park=[16])
    monkeypatch.setattr(session, "residual", lambda: ["emulated gear still registered: 16"])
    assert session.close() == ["emulated gear still registered: 16"]
    assert session.ledger.exists()


def test_a_residue_check_that_fails_keeps_the_ledger(tmp_path, monkeypatch):
    session = _session(tmp_path, _TeardownApi(), _QuietSim(), park=[16])

    def residual():
        raise OSError("ssh to the WB timed out")
    monkeypatch.setattr(session, "residual", residual)
    assert any("residue check failed" in line for line in session.close())
    assert session.ledger.exists()


def test_the_restore_command_tears_a_left_session_down_without_an_emulator(tmp_path, monkeypatch):
    cfg = type("Cfg", (), {"state_dir": tmp_path, "peer": lambda self: None})()
    virtual_gear.Ledger.of(cfg).update(park=[16])

    def no_console(peer):
        raise virtual_gear.GearSimUnavailable("the peer runs the controller")
    monkeypatch.setattr(virtual_gear, "GearSim", no_console)
    monkeypatch.setattr(virtual_gear.VirtualSession, "residual", lambda self: [])
    api = _TeardownApi()
    assert virtual_gear.teardown(cfg, api, log=lambda line: None) == []
    assert api.calls == [("forget", 16)] and not virtual_gear.Ledger.of(cfg).exists()


def test_no_ledger_means_nothing_to_tear_down(tmp_path):
    cfg = type("Cfg", (), {"state_dir": tmp_path})()
    assert virtual_gear.teardown(cfg, _TeardownApi()) == []


class _EnrolApi:
    def __init__(self):
        self.vlamps, self.bound = self, []

    def wait_op(self, op):
        return op

    def discovery(self, mode):
        return {"operation_id": 1}

    def devices_unfiltered(self):
        return {"physical_devices": [{"short_address": 16}]}

    def patch(self, lamp_id, body):
        raise RuntimeError("the controller refused the VL")

    def bind(self, lamp_id, short):
        self.bound.append((lamp_id, short))


class _ShowSim:
    def show(self):
        return [{"short": 16, "groups": 0}]


def test_a_vl_is_ledgered_before_it_is_created(tmp_path):
    session = _session(tmp_path, _EnrolApi(), _ShowSim(), vl_before=[0, 1])
    with pytest.raises(RuntimeError):
        session._enrol([16], [4])
    assert session.ledger.data["created_vls"] == [63]


class _LadderSim:
    def __init__(self, moves):
        self.snapshots, self.enabled = [dict(sent=5), dict(sent=5 + moves[0], late=moves[1])], []

    def stats(self):
        return self.snapshots.pop(0)

    def enable(self, which):
        self.enabled.append(which)


class _LadderApi:
    def __init__(self, reply):
        self.reply = reply

    def cmd(self, short, opcode):
        return self.reply


def test_the_first_rung_counts_its_own_answer(tmp_path):
    sim = _LadderSim((1, 0))
    _session(tmp_path, _LadderApi({"success": True, "backward_frame": 0}), sim)._ladder(
        [16, 17, 18, 19, 20])
    assert sim.enabled == [16, 17, 18, 19, "all"]


@pytest.mark.parametrize("reply,moves", [
    ({"success": True, "backward_violation": True}, (1, 0)),
    ({"success": True, "backward_frame": 0}, (0, 0)),
    ({"success": True, "backward_frame": 0}, (1, 1)),
])
def test_a_first_rung_that_proves_nothing_stops_the_ladder(tmp_path, reply, moves):
    sim = _LadderSim(moves)
    with pytest.raises(virtual_gear.VirtualGearError):
        _session(tmp_path, _LadderApi(reply), sim)._ladder([16, 17])
    assert sim.enabled == [16]
