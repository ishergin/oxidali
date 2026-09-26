import pytest

from hil import tripwire, virtual_gear
from hil.lamp_guard import LampGuard, LampNotAllowed, VirtualFence

WB_CONF = {"gateways": [{"device_id": "wb-dali_19", "buses": [
    {"devices": [{"short": 0}, {"short": 1}, {"short": 12}, {"short": 0, "dali2": True}]},
    {"devices": [{"short": 30}]}]}]}

OWNER_RULES = ('rule "подсветка: включить" {\n  when group(0) becomes any_on\n'
               '  do lamp("Подсветка").on()\n}\nrule "кнопка 3" {\n  when input(dev=0, inst=2)'
               ' is short_press\n}\n')


def test_the_reserve_covers_the_floor_and_every_known_address():
    reserved = virtual_gear.reserve({1, 4, 12}, {0, 13}, {20})
    assert set(range(16)) <= reserved and {20} <= reserved
    assert virtual_gear.spell(reserved) == "0-15,20"


def test_the_park_takes_the_first_free_addresses():
    assert virtual_gear.park_shorts(frozenset(range(16)) | {17}, 3) == [16, 18, 19]
    with pytest.raises(virtual_gear.VirtualGearError):
        virtual_gear.park_shorts(frozenset(range(62)), 3)


def test_the_wb_list_is_read_for_the_named_gateway_and_bus_without_part_103_units():
    assert virtual_gear.wb_shorts_of(WB_CONF, "wb-dali_19", 1) == {0, 1, 12}
    assert virtual_gear.wb_shorts_of(WB_CONF, "wb-dali_19", 2) == {30}
    assert virtual_gear.wb_shorts_of(WB_CONF, "other", 1) == set()


def test_free_groups_leave_out_applied_and_desired_ones():
    devices = [{"groups_membership": 0b0001}, {"groups_membership": 0b0100}]
    rows = [{"desired": [False, True] + [False] * 14}]
    assert virtual_gear.free_groups(virtual_gear.used_groups(devices, rows)) == \
        list(range(3, 16))


def test_an_unread_membership_frees_no_group():
    devices = [{"groups_membership": 1}, {"groups_membership": None}]
    assert virtual_gear.free_groups(virtual_gear.used_groups(devices, [])) == []


class _WireApi:
    def __init__(self, answering):
        self.answering, self.sent = set(answering), []

    def cmd_wire(self, addr, opcode):
        self.sent.append((addr, opcode))
        group = (addr >> 1) & 0x0F
        return {"success": group in self.answering, "backward_frame": 0xFF
                if group in self.answering else 0}


def test_free_groups_are_proven_silent_after_a_positive_control():
    api = _WireApi({0, 2})
    assert virtual_gear.prove_groups_empty(api, [4, 5], {0, 2}) == 0
    assert (0x81, 0x91) in api.sent and (0x89, 0x91) in api.sent


def test_silence_without_a_positive_control_proves_nothing():
    with pytest.raises(virtual_gear.VirtualGearError, match="prove nothing"):
        virtual_gear.prove_groups_empty(_WireApi(set()), [4], {0})


def test_a_free_group_that_answers_is_occupied():
    with pytest.raises(virtual_gear.VirtualGearError, match=r"\[5\]"):
        virtual_gear.prove_groups_empty(_WireApi({0, 5}), [4, 5], {0})


def test_a_violating_answer_counts_as_present():
    assert virtual_gear.answered_yes({"success": False, "backward_violation": True})
    assert not virtual_gear.answered_yes({"success": False, "backward_frame": 0})


def test_the_owner_rules_may_not_use_the_session_groups_or_names():
    assert virtual_gear.rules_conflicts(OWNER_RULES, [0, 4], ["x"]) == \
        ["the owner's rules use group 0"]
    assert virtual_gear.rules_conflicts(OWNER_RULES, [4], ["Подсветка"]) == \
        ["the owner's rules name lamp 'Подсветка'"]
    assert virtual_gear.rules_conflicts(OWNER_RULES, [4, 5], ["virtual gear SA16"]) == []


def test_session_vls_take_ids_nobody_holds():
    assert virtual_gear.free_vl_ids([0, 1, 63], 2) == [61, 62]


def _fence(**kw):
    args = dict(park=[16, 17], groups=[4, 5], session_vls=[60, 61],
                owner_rules=OWNER_RULES, owner_schedules=["moscow-cct"])
    args.update(kw)
    return VirtualFence(**args)


def _guard(fence):
    guard = LampGuard({16, 17}, segment=lambda: [1, 4, 16, 17])
    guard.fence = fence
    return guard


@pytest.mark.parametrize("method,path,body", [
    ("PUT", "adapters/0/groups/4/target-state", {"power": "on"}),
    ("POST", "adapters/0/scenes/3/recall", {"scope": "group", "group_id": 5}),
    ("POST", "hcl-schedules", {"targets": [{"scope": "group", "group_ids": [4]}]}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 60, "desired": [False] * 4 + [True] + [False] * 11}]}),
    ("PUT", "adapters/0/virtual-lamps/60/binding", {"physical_short_address": 16}),
    ("POST", "adapters/0/physical-devices/16/write-attributes", {"fade_time_ms": 0}),
    ("PUT", "adapters/0/physical-devices/16/target-state", {"level": 100}),
    ("POST", "adapters/0/discovery-runs", {"mode": "scan_known_short_addresses"}),
    ("PUT", "rules", {"source": OWNER_RULES + 'rule "t" { when group(4) becomes any_on }'}),
    ("POST", "rules/t/run", None),
])
def test_the_fence_lets_the_session_reach_its_own_gear(method, path, body):
    _guard(_fence()).check_request(method, path, body)


@pytest.mark.parametrize("method,path,body", [
    ("PUT", "adapters/0/groups/0/target-state", {"power": "on"}),
    ("POST", "adapters/0/scenes/3/recall", None),
    ("POST", "adapters/0/scenes/3/recall", {"scope": "group", "group_id": 0}),
    ("POST", "hcl-schedules", {"targets": [{"scope": "broadcast"}]}),
    ("PATCH", "hcl-schedules/x", {"targets": [{"scope": "group", "group_ids": [4, 1]}]}),
    ("DELETE", "hcl-schedules/moscow-cct", None),
    ("PUT", "adapters/0/group-membership-matrix", {"rows": []}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 6, "desired": [True] + [False] * 15}]}),
    ("PATCH", "adapters/0/group-membership-matrix",
     {"rows": [{"virtual_lamp_id": 60, "desired": [True] + [False] * 15}]}),
    ("PATCH", "adapters/0/groups/4", {"name": "x"}),
    ("PATCH", "adapters/0/scenes/2", {"name": "x"}),
    ("PUT", "adapters/0/virtual-lamps/6/binding", {"physical_short_address": 16}),
    ("PUT", "adapters/0/virtual-lamps/60/binding", {"physical_short_address": 6}),
    ("DELETE", "adapters/0/virtual-lamps/6", None),
    ("DELETE", "adapters/0/physical-devices/6", None),
    ("POST", "adapters/0/physical-devices/6/write-attributes", {"fade_time_ms": 0}),
    ("PATCH", "policies", {"apply_on_discovery": True}),
    ("POST", "adapters/0/commissioning/steps/initialise", {"scope": "unaddressed"}),
    ("POST", "adapters/0/discovery-runs", {"mode": "commission_unaddressed"}),
    ("PUT", "rules", {"source": 'rule "t" { when group(1) becomes any_on }'}),
    ("POST", "rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203/run", None),
    ("PATCH", "rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203", {"enabled": False}),
])
def test_the_fence_refuses_what_reaches_past_the_session(method, path, body):
    with pytest.raises(LampNotAllowed):
        _guard(_fence()).check_request(method, path, body)


def test_an_apply_is_allowed_only_when_its_diff_is_the_sessions():
    pending = {"rows": {60}}
    fence = _fence(pending=lambda kind, scene: pending["rows"])
    _guard(fence).check_request("POST", "adapters/0/groups/apply", None)
    pending["rows"] = {60, 6}
    with pytest.raises(LampNotAllowed, match="VL6"):
        _guard(fence).check_request("POST", "adapters/0/scenes/1/apply", None)


def test_commissioning_needs_its_flag():
    fence = _fence(commissioning=True)
    _guard(fence).check_request("POST", "adapters/0/commissioning/steps/initialise", {})
    _guard(fence).check_frame(0xA5, 0xFF)
    with pytest.raises(LampNotAllowed):
        _guard(_fence()).check_frame(0xA7, 0x00)


@pytest.mark.parametrize("addr,data,refused", [
    (0x89, 0x05, False),
    (0x81, 0x05, True),
    (0xFE, 0x80, True),
    (0xFF, 0x05, True),
    (0x21, 0x64, False),
    (0x21, 0x60, True),
    (0x21, 0x74, False),
    (0x0D, 0x05, True),
    (0x81, 0x91, False),
])
def test_the_fence_judges_frames(addr, data, refused):
    guard = _guard(_fence())
    if refused:
        with pytest.raises(LampNotAllowed):
            guard.check_frame(addr, data)
    else:
        guard.check_frame(addr, data)


def test_the_tripwire_reads_only_frames_actually_sent():
    lines = ["I (1) x: DALI PHY TX: forward16 0x2105 (ISR-owned bitbang)",
             "W (2) x: DALI PHY TX: collision on forward16 0xfe00",
             "I (3) x: DALI PHY TX: forward24 0xfffe12 (ISR-owned bitbang)"]
    assert tripwire.sent_frames(lines) == [(0x21, 0x05)]


def test_the_tripwire_names_every_frame_outside_the_session():
    lines = ["DALI PHY TX: forward16 0x%04x (ISR-owned bitbang)" % f
             for f in (0x2105, 0x8905, 0x8105, 0xfe80, 0x0d05, 0x0d91, 0xa500)]
    found = tripwire.violations(lines, park=[16], groups=[4])
    assert len(found) == 3
    assert any("group" in v for v in found) and any("broadcast" in v for v in found)
    assert any("SA6" in v for v in found)


def test_lost_log_lines_are_reported():
    before = {"console_log_dropped_total": 3, "console_log_busy_total": 0}
    after = {"console_log_dropped_total": 5, "console_log_busy_total": 0}
    assert tripwire.lost_lines(before, after) == {"console_log_dropped_total": 2}


class _TeardownApi:
    def __init__(self):
        self.calls = []
        self.vlamps = self
        self.groups = self

    def delete(self, lamp_id):
        self.calls.append(("vl-delete", lamp_id))

    def device_forget(self, short):
        self.calls.append(("forget", short))

    def patch(self, group, body):
        self.calls.append(("group", group, body))

    def _req(self, method, path, body=None):
        self.calls.append((method, path, body))
        return {}


class _QuietSim:
    def __init__(self):
        self.disabled = []

    def disable(self, which):
        self.disabled.append(which)


def test_teardown_deletes_vls_before_devices_and_restores_flags_and_policy(tmp_path, monkeypatch):
    cfg = type("Cfg", (), {"state_dir": tmp_path})()
    api, sim = _TeardownApi(), _QuietSim()
    session = virtual_gear.VirtualSession(cfg, api, None, sim)
    session.ledger.update(park=[16, 17], created_vls=[60, 61], group_flags={"4": True},
                          policy_rearm=True)
    monkeypatch.setattr(session, "residual", lambda: [])
    assert session.close() == []
    assert sim.disabled == ["all"]
    assert api.calls == [("vl-delete", 60), ("vl-delete", 61), ("forget", 16), ("forget", 17),
                         ("group", 4, {"ha_entity_enabled": True}),
                         ("PATCH", "policies", {"apply_on_discovery": True})]
    assert not session.ledger.exists()


def test_a_teardown_with_residue_keeps_its_ledger(tmp_path, monkeypatch):
    cfg = type("Cfg", (), {"state_dir": tmp_path})()
    session = virtual_gear.VirtualSession(cfg, _TeardownApi(), None, _QuietSim())
    session.ledger.update(park=[16])
    monkeypatch.setattr(session, "residual", lambda: ["emulated gear still registered: 16"])
    assert session.close() == ["emulated gear still registered: 16"]
    assert session.ledger.exists()
