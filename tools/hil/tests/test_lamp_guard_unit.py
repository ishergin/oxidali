import dataclasses
import re

import pytest

import hil.api
import test_pd_forget
from hil import pair, write_log
from hil.config import HilConfig, load as load_config
from hil.foreign import ForeignMaster, injection_refusal
from hil.lamp_guard import LampGuard, LampNotAllowed

LAMPS = frozenset({0, 2, 3})
OWNER = 1
REMOVE_FROM_SCENE_12 = 0x50 + 12
QUERY_ACTUAL_LEVEL = 0xA0
ADD_TO_GROUP_5 = 0x60 + 5


def _short_wire(short, command=False):
    return (short << 1) | (1 if command else 0)


def _guard(segment=(0, 2, 3), read_only=False, policy_armed=False):
    return LampGuard(LAMPS, read_only=read_only, segment=lambda: list(segment),
                     policy_armed=lambda: policy_armed)


def test_a_short_outside_the_allowlist_is_refused_and_named():
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside HIL_LAMP_SHORTS=0,2-3"):
        _guard().check_frame(_short_wire(OWNER), 100)
    with pytest.raises(LampNotAllowed, match=r"wire address 0x03"):
        _guard().check_frame(_short_wire(OWNER, command=True), REMOVE_FROM_SCENE_12)
    _guard().check_frame(_short_wire(2), 100)


def test_a_group_frame_on_a_partly_allowed_segment_is_refused():
    guard = _guard(segment=(0, 1, 2, 3))
    with pytest.raises(LampNotAllowed, match=r"whole segment, and SA1 is outside"):
        guard.check_frame(0x80 | (3 << 1), 100)
    with pytest.raises(LampNotAllowed, match=r"broadcast"):
        guard.check_frame(0xFF, REMOVE_FROM_SCENE_12)


def test_a_broadcast_on_a_fully_allowed_segment_passes():
    guard = _guard(segment=(0, 2, 3))
    guard.check_frame(0xFE, 100)
    guard.check_frame(0xFF, REMOVE_FROM_SCENE_12)


def test_a_segment_nobody_can_list_refuses_a_broadcast():
    with pytest.raises(LampNotAllowed, match=r"no controller lists"):
        LampGuard(LAMPS).check_frame(0xFE, 100)


def test_a_query_passes_whatever_the_target_and_the_mode():
    guard = _guard(segment=(0, 1, 2, 3), read_only=True)
    guard.check_frame(_short_wire(OWNER, command=True), QUERY_ACTUAL_LEVEL)
    guard.check_frame(0xFF, QUERY_ACTUAL_LEVEL)
    guard.check_frame(0xA3, 0x72)


def test_read_only_refuses_a_visible_action_and_keeps_a_configuration_write():
    guard = _guard(read_only=True)
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_frame(_short_wire(0), 100)
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_request("PUT", "adapters/0/physical-devices/0/target-state",
                            {"power": "off"})
    guard.check_frame(_short_wire(0, command=True), ADD_TO_GROUP_5)


ENABLE_DEVICE_TYPE = 0xC1
SELECT_DIMMING_CURVE = 0xE3
STORE_TC_LIMIT = 0xF2
STORE_GEAR_FEATURES = 0xF3
QUERY_COLOUR_VALUE = 0xFA


def test_the_client_reads_an_extended_command_by_the_enable_it_sent_before():
    guard = _guard(read_only=True)
    guard.check_frame(ENABLE_DEVICE_TYPE, 6)
    with pytest.raises(LampNotAllowed, match=r"\(SA1\) refused: HIL_LAMPS_READ_ONLY"):
        guard.check_frame(_short_wire(OWNER, command=True), SELECT_DIMMING_CURVE)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    guard.check_frame(_short_wire(OWNER, command=True), QUERY_COLOUR_VALUE)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    guard.check_frame(_short_wire(2, command=True), STORE_GEAR_FEATURES)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_frame(_short_wire(2, command=True), 0xE2)


def test_a_refused_frame_leaves_the_enable_it_followed_in_force():
    guard = _guard()
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    for _ in range(2):
        with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
            guard.check_frame(_short_wire(OWNER, command=True), 0xE2)


def test_an_extended_frame_with_no_enable_this_client_sent_is_an_unknown_write(monkeypatch):
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        _guard(read_only=True).check_request(
            "POST", "dali/raw", {"frame": (_short_wire(2, command=True) << 8) | 0xE3})
    guard = _guard()
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    guard.check_frame(_short_wire(2, command=True), QUERY_COLOUR_VALUE)
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
        guard.check_frame(_short_wire(OWNER, command=True), SELECT_DIMMING_CURVE)
    guard.check_frame(_short_wire(OWNER, command=True), 0xFF)
    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="0,2,3",
                                               lamps_read_only=False))
    sent = []
    monkeypatch.setattr(master, "_run_client", lambda frames: sent.append(frames) or [])
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
        master.raw16(_short_wire(OWNER, command=True), 0xE2)
    assert sent == []


FORBIDDEN_SA5 = 5
GROUP_0_COMMAND = 0x81
BROADCAST_COMMAND = 0xFF
DISABLE_CURRENT_PROTECTOR = 0xE2
Y_STEP_UP = 0xE5


def _typed(wire_address, command):
    return {"wire_address": wire_address, "command": command, "repeat_count": 1}


def test_a_typed_extended_command_is_judged_by_the_enable_the_firmware_sends_for_it():
    nothing_allowed = LampGuard((), read_only=True)
    for wire_address in (_short_wire(FORBIDDEN_SA5, command=True), GROUP_0_COMMAND,
                         BROADCAST_COMMAND):
        with pytest.raises(LampNotAllowed):
            nothing_allowed.check_request("POST", "dali/command",
                                          _typed(wire_address, SELECT_DIMMING_CURVE))
    nothing_allowed.check_request("POST", "dali/command",
                                  _typed(_short_wire(FORBIDDEN_SA5, command=True),
                                         QUERY_COLOUR_VALUE))
    client = _client()
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
        client.cmd(OWNER, DISABLE_CURRENT_PROTECTOR)
    assert client.http.sent == []
    guard = _guard(read_only=True)
    guard.check_request("POST", "dali/command",
                        _typed(_short_wire(2, command=True), DISABLE_CURRENT_PROTECTOR))
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_request("POST", "dali/command",
                            _typed(_short_wire(2, command=True), Y_STEP_UP))
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
        _guard().check_request("POST", "dali/command",
                               _typed(_short_wire(OWNER, command=True), Y_STEP_UP))


REFERENCE_SYSTEM_POWER = 0xE0
STORE_DTR_AS_FAST_FADE_TIME = 0xE4
SET_TEMPORARY_X = 0xE0
X_STEP_UP, Y_STEP_DOWN, TC_STEP_COOLER, TC_STEP_WARMER = 0xE3, 0xE6, 0xE8, 0xE9
START_AUTO_CALIBRATION = 0xF6
UNCLASSIFIED_DEVICE_TYPE = 7


def test_read_only_counts_every_extended_command_that_moves_the_light_as_visible():
    guard = _guard(read_only=True)
    for enabled, opcode in ((6, REFERENCE_SYSTEM_POWER), (6, SELECT_DIMMING_CURVE),
                            (8, X_STEP_UP), (8, Y_STEP_DOWN), (8, TC_STEP_COOLER),
                            (8, TC_STEP_WARMER), (8, START_AUTO_CALIBRATION), (8, 0xE2),
                            (8, STORE_TC_LIMIT),
                            (UNCLASSIFIED_DEVICE_TYPE, REFERENCE_SYSTEM_POWER)):
        guard.check_frame(ENABLE_DEVICE_TYPE, enabled)
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
            guard.check_frame(_short_wire(2, command=True), opcode)
    for enabled, opcode in ((6, STORE_DTR_AS_FAST_FADE_TIME), (6, DISABLE_CURRENT_PROTECTOR),
                            (8, SET_TEMPORARY_X), (8, STORE_GEAR_FEATURES)):
        guard.check_frame(ENABLE_DEVICE_TYPE, enabled)
        guard.check_frame(_short_wire(2, command=True), opcode)


DTR0 = 0xA3
SET_MAX_LEVEL, SET_MIN_LEVEL, SET_FADE_TIME = 0x2A, 0x2B, 0x2E
CLAMPED_LEVEL = 100
WRITE_ATTRIBUTES = "adapters/0/physical-devices/%d/write-attributes"


def _raw(addr, data):
    return {"frame": (addr << 8) | data}


def test_a_new_level_limit_is_a_visible_frame_and_reaches_only_an_allowed_lamp():
    for opcode in (SET_MAX_LEVEL, SET_MIN_LEVEL):
        read_only = _guard(read_only=True)
        read_only.check_request("POST", "dali/raw", _raw(DTR0, CLAMPED_LEVEL))
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
            read_only.check_request("POST", "dali/raw", _raw(_short_wire(2, command=True),
                                                             opcode))
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
            read_only.check_request("POST", "dali/command",
                                    _typed(_short_wire(2, command=True), opcode))
        driving = _guard()
        driving.check_request("POST", "dali/raw", _raw(DTR0, CLAMPED_LEVEL))
        driving.check_request("POST", "dali/raw", _raw(_short_wire(2, command=True), opcode))
        with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
            driving.check_request("POST", "dali/raw", _raw(_short_wire(OWNER, command=True),
                                                           opcode))
    _guard(read_only=True).check_frame(_short_wire(2, command=True), SET_FADE_TIME)


def test_an_attribute_write_that_moves_a_lit_lamp_is_visible_and_reaches_only_an_allowed_lamp():
    read_only, driving = _guard(read_only=True), _guard()
    for body in ({"max_level": CLAMPED_LEVEL}, {"min_level": CLAMPED_LEVEL},
                 {"fade_time_ms": 0, "max_level": CLAMPED_LEVEL}, {"dimming_curve": 1},
                 {"tc_coolest_mirek": 153}, {"tc_warmest_mirek": 370}):
        with pytest.raises(LampNotAllowed, match=r"attribute write to SA2 refused: "
                                                 r"HIL_LAMPS_READ_ONLY"):
            read_only.check_request("POST", WRITE_ATTRIBUTES % 2, body)
        driving.check_request("POST", WRITE_ATTRIBUTES % 2, body)
        with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
            driving.check_request("POST", WRITE_ATTRIBUTES % OWNER, body)
    read_only.check_request("POST", WRITE_ATTRIBUTES % 2, {"fade_time_ms": 0,
                                                           "power_on_level": 254})


def test_an_attribute_write_reaches_only_an_allowed_lamp_and_is_not_a_visible_action():
    guard = _guard(read_only=True)
    guard.check_request("POST", "adapters/0/physical-devices/2/write-attributes",
                        {"fade_time_ms": 0})
    with pytest.raises(LampNotAllowed, match=r"attribute write to SA1 refused: SA1 is outside"):
        guard.check_request("POST", "adapters/0/physical-devices/1/write-attributes",
                            {"fade_time_ms": 0})
    guard.check_request("POST", "adapters/0/physical-devices/1/attribute-reads",
                        {"attribute_groups": ["common_102"]})


def test_the_range_form_parses_and_bounds_the_optical_set():
    cfg = HilConfig(lamp_shorts="0-3", optical_shorts="")
    assert cfg.lamp_short_set() == frozenset({0, 1, 2, 3})
    assert HilConfig(lamp_shorts="0,2,3", optical_shorts="0-3").optical_short_set() == LAMPS
    assert HilConfig(lamp_shorts="", optical_shorts="").lamp_short_set() == frozenset()


class _Response:
    status_code = 200
    content = b"{}"
    text = ""

    def __init__(self, payload):
        self._payload = payload

    def json(self):
        return self._payload


class _Session:
    def __init__(self, shorts, bindings=None, rules=(), policy=None):
        self.shorts = shorts
        self.bindings = bindings or {}
        self.rules = list(rules)
        self.policy = policy or {}
        self.sent = []

    def request(self, method, url, json=None, timeout=None):
        path = url.split("/api/v1/", 1)[1]
        if method != "GET":
            self.sent.append((method, path, json))
            return _Response({})
        if path.endswith("/physical-devices"):
            return _Response({"physical_devices": [
                {"short_address": s, "present": True} for s in self.shorts]})
        if path == "rules?format=json":
            return _Response({"rules": {"rules": self.rules}})
        if path == "policies":
            return _Response(self.policy)
        lamp = re.search(r"virtual-lamps/(\d+)$", path)
        if lamp:
            return _Response({"binding": {
                "physical_short_address": self.bindings.get(int(lamp.group(1)))}})
        return _Response({})

    def close(self):
        pass


def _client(shorts=(0, 1, 2, 3), bindings=None, read_only=False, rules=(), policy=None):
    cfg = dataclasses.replace(load_config(), lamp_shorts="0,2,3", gear_shorts="",
                              lamps_read_only=read_only)
    client = hil.api.Client(cfg)
    client.http = _Session(list(shorts), bindings, rules, policy)
    return client


def test_every_client_door_to_a_forbidden_lamp_is_shut_before_the_wire(monkeypatch):
    monkeypatch.setattr(hil.api.time, "sleep", lambda _s: None)
    client = _client(bindings={7: 4, 8: 2})
    doors = (
        lambda: client.ts(OWNER, {"power": "on"}),
        lambda: client.off(OWNER),
        lambda: client.dapc(OWNER, 100),
        lambda: client.cmd(OWNER, REMOVE_FROM_SCENE_12),
        lambda: client.cmd_wire(0xFF, REMOVE_FROM_SCENE_12),
        lambda: client.raw((0xFE << 8) | 100),
        lambda: client.identify(OWNER),
        lambda: client.group_ts(3, {"power": "on"}),
        lambda: client.scenes.recall(12),
        lambda: client.scenes.recall(12, group_id=3),
        lambda: client.vlamps.ts(7, {"power": "on"}),
        lambda: client.raw_request("POST", "dali/level",
                                   {"wire_address": _short_wire(OWNER), "level": 9}),
        lambda: client.raw_request("POST", "dali/raw", {"frame": "not a frame"}),
        lambda: client.raw_response("PUT", "adapters/0/physical-devices/1/target-state",
                                    {"power": "on"}),
        lambda: client.raw_response("POST", "dali/command",
                                    {"wire_address": 0xFF, "command": 0x10}),
        lambda: client.device_forget(OWNER),
    )
    for door in doors:
        with pytest.raises(LampNotAllowed):
            door()
    assert client.http.sent == []
    client.vlamps.ts(8, {"power": "on"})
    client.cmd(OWNER, QUERY_ACTUAL_LEVEL)
    assert [path for _m, path, _b in client.http.sent] == [
        "adapters/0/virtual-lamps/8/target-state", "dali/command"]


class _Failing(_Response):
    status_code = 500


def test_a_virtual_lamp_target_is_refused_unless_its_binding_was_read(monkeypatch):
    client = _client()
    answer = client.http.request

    def request(method, url, json=None, timeout=None):
        if method == "GET" and url.endswith("virtual-lamps/9"):
            return _Failing({"error": "busy"})
        return answer(method, url, json=json, timeout=timeout)

    monkeypatch.setattr(client.http, "request", request)
    with pytest.raises(LampNotAllowed, match=r"VL9 refused: its binding could not be read"):
        client.vlamps.ts(9, {"power": "on"})
    assert client.http.sent == []
    client.vlamps.ts(5, {"power": "on"})
    assert [path for _m, path, _b in client.http.sent] == [
        "adapters/0/virtual-lamps/5/target-state"]


def test_read_only_refuses_a_visible_route_before_it_resolves_the_target():
    looked = []
    guard = LampGuard(LAMPS, read_only=True, binding=looked.append)
    with pytest.raises(LampNotAllowed, match=r"target-state of VL7 refused: "
                                             r"HIL_LAMPS_READ_ONLY"):
        guard.check_request("PUT", "adapters/0/virtual-lamps/7/target-state", {"power": "on"})
    assert looked == []
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_request("POST", "adapters/0/commissioning/identify", {})
    with pytest.raises(LampNotAllowed, match=r"names no short address"):
        LampGuard(LAMPS).check_request("POST", "adapters/0/commissioning/identify", {})
    with pytest.raises(LampNotAllowed, match=r"no controller resolves"):
        LampGuard(LAMPS).check_request("PUT", "adapters/0/virtual-lamps/7/target-state",
                                       {"power": "on"})


COMMISSIONING_REQUESTS = (
    ("adapters/0/commissioning/steps/initialise", {"scope": "unaddressed"}),
    ("adapters/0/commissioning/steps/program-short-address", {"short_address": 2}),
    ("adapters/0/commissioning/address-changes", {"from": 2, "to": 3}),
    ("adapters/0/commissioning/replacements", {"short_address": 2}),
    ("adapters/0/input-devices/commission", {"include_addressed": False}),
    ("adapters/0/discovery-runs", {"mode": "commission_unaddressed"}),
    ("adapters/0/discovery-runs", {}),
)
INITIALISE, RANDOMISE, PROGRAM_SHORT_ADDRESS, RESERVED_SPECIAL = 0xA5, 0xA7, 0xB7, 0xCB


def test_commissioning_never_passes_the_guard_outside_the_virtual_tier(monkeypatch):
    guard = _guard()
    for path, body in COMMISSIONING_REQUESTS:
        with pytest.raises(LampNotAllowed, match=r"commissioning never runs on the installation"):
            guard.check_request("POST", path, body)
    for mode in ("scan_known_short_addresses", "refresh_known"):
        guard.check_request("POST", "adapters/0/discovery-runs", {"mode": mode})
    for addr, data in ((INITIALISE, 0xFF), (RANDOMISE, 0), (PROGRAM_SHORT_ADDRESS, 5)):
        with pytest.raises(LampNotAllowed, match=r"commissioning never runs"):
            guard.check_request("POST", "dali/raw", {"frame": (addr << 8) | data})
        with pytest.raises(LampNotAllowed, match=r"commissioning never runs"):
            guard.check_request("POST", "dali/command", _typed(addr, data))
    with pytest.raises(LampNotAllowed, match=r"does not know what it does"):
        guard.check_frame(RESERVED_SPECIAL, 0)
    guard.check_frame(DTR0, 0x10)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="0,2,3",
                                               lamps_read_only=False))
    sent = []
    monkeypatch.setattr(master, "_run_client", lambda frames: sent.append(frames) or [])
    with pytest.raises(LampNotAllowed, match=r"commissioning never runs"):
        master.raw16(INITIALISE, 0x00)
    assert sent == []


SCAN = ("POST", "adapters/0/discovery-runs", {"mode": "refresh_known"})
ARMED = {"apply_on_discovery": True, "manages_anything": True}


def test_a_scan_with_apply_on_discovery_armed_must_be_allowed_every_gear_on_the_segment():
    _guard(segment=(0, OWNER, 2, 3)).check_request(*SCAN)
    with pytest.raises(LampNotAllowed, match=r"discovery-runs with apply-on-discovery armed, "
                                             r"which writes power_on_level .* SA1 is outside"):
        _guard(segment=(0, OWNER, 2, 3), policy_armed=True).check_request(*SCAN)
    _guard(policy_armed=True).check_request(*SCAN)
    _guard(policy_armed=True, read_only=True).check_request(*SCAN)


def test_a_policy_apply_must_be_allowed_every_gear_on_the_segment_whatever_the_flag():
    with pytest.raises(LampNotAllowed, match=r"POST policies/apply, which writes .* SA1 is "
                                             r"outside"):
        _guard(segment=(0, OWNER, 2, 3)).check_request("POST", "policies/apply", {})
    _guard().check_request("POST", "policies/apply", {})


def test_a_scan_is_refused_when_no_controller_says_whether_the_policy_is_armed():
    with pytest.raises(LampNotAllowed, match=r"no controller says whether apply-on-discovery"):
        LampGuard(LAMPS, segment=lambda: [0, 2, 3]).check_request(*SCAN)


def test_the_client_asks_the_controller_whether_a_scan_writes_the_policy():
    armed = _client(policy=ARMED)
    with pytest.raises(LampNotAllowed, match=r"apply-on-discovery armed"):
        armed.discovery("refresh_known")
    assert armed.http.sent == []
    managing_nothing = _client(policy=dict(ARMED, manages_anything=False))
    managing_nothing.discovery("refresh_known")
    assert managing_nothing.http.sent == [SCAN]


GROUP_APPLY, SCENE_APPLY = "adapters/0/groups/apply", "adapters/0/scenes/3/apply"


def _applying(read_only=False, pending=(), bindings=None):
    bindings = bindings or {}
    return LampGuard(LAMPS, read_only=read_only, segment=lambda: [0, 2, 3],
                     binding=bindings.get, pending=lambda kind, scene: set(pending))


def test_an_apply_writes_only_rows_of_lamps_the_guard_allows():
    for path in (GROUP_APPLY, SCENE_APPLY):
        _applying(pending={7}, bindings={7: 2}).check_request("POST", path)
        _applying().check_request("POST", path)
        with pytest.raises(LampNotAllowed, match=r"SA1 is outside"):
            _applying(pending={7, 8}, bindings={7: 2, 8: OWNER}).check_request("POST", path)
        with pytest.raises(LampNotAllowed, match=r"VL9, which is bound to no lamp"):
            _applying(pending={9}).check_request("POST", path)
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
            _applying(read_only=True).check_request("POST", path)
        with pytest.raises(LampNotAllowed, match=r"no controller lists"):
            LampGuard(LAMPS).check_request("POST", path)


def test_the_client_refuses_an_apply_that_would_push_an_owner_row(monkeypatch):
    client = _client(bindings={7: OWNER})
    rows = [{"virtual_lamp_id": 7, "desired": [True] + [False] * 15, "applied": [False] * 16}]
    answer = client.http.request

    def request(method, url, json=None, timeout=None):
        if method == "GET" and url.endswith("group-membership-matrix"):
            return _Response({"rows": rows})
        return answer(method, url, json=json, timeout=timeout)

    monkeypatch.setattr(client.http, "request", request)
    with pytest.raises(LampNotAllowed, match=r"row of VL7 refused: SA1 is outside"):
        client.groups.apply()
    assert client.http.sent == []


PANEL, KEYED_INSTANCE, FREE_INSTANCE = 5, 0, 3


def _input_rule(name, trigger, enabled=True):
    return {"name": name, "enabled": enabled, "triggers": [trigger]}


OWNER_INPUT_RULES = {"rules": [
    _input_rule("hall", {"kind": "input_event", "adapter_id": 0, "device_short_address": PANEL,
                         "instance_number": KEYED_INSTANCE, "event": "short_press"}),
    _input_rule("boot", {"kind": "input_device_power_cycled", "adapter_id": 0,
                         "device_short_address": PANEL}),
    _input_rule("hil-inp-06", {"kind": "input_event", "adapter_id": 0,
                               "device_short_address": PANEL, "instance_number": FREE_INSTANCE,
                               "event": "short_press"}),
    _input_rule("off", {"kind": "input_event", "adapter_id": 0, "device_short_address": PANEL,
                        "instance_number": FREE_INSTANCE, "event": "press"}, enabled=False),
]}
GROUP_RULES = {"rules": [_input_rule("any", {"kind": "input_event", "adapter_id": 0,
                                             "instance_group": 7, "event": "press"})]}


def _event(short, instance, info=0x002):
    return [short << 1, 0x80 | (instance << 2) | (info >> 8), info & 0xFF]


def test_an_injected_event_is_refused_when_an_owner_rule_could_fire_on_it(monkeypatch):
    assert "hall" in injection_refusal(_event(PANEL, KEYED_INSTANCE), OWNER_INPUT_RULES, 0)
    assert injection_refusal(_event(PANEL, FREE_INSTANCE), OWNER_INPUT_RULES, 0) is None
    assert "boot" in injection_refusal([0xFE, 0xE0, 0x40 | PANEL], OWNER_INPUT_RULES, 0)
    assert injection_refusal([0xFE, 0xE0, 0x40 | (PANEL + 1)], OWNER_INPUT_RULES, 0) is None
    assert injection_refusal([0x80 | (1 << 1), 0x80, 0x002], OWNER_INPUT_RULES, 0) is None
    assert "any" in injection_refusal(_event(PANEL + 1, FREE_INSTANCE), GROUP_RULES, 0)
    assert "any" in injection_refusal([0xC0 | (7 << 1), 0x00, 0x001], GROUP_RULES, 0)
    assert "command" in injection_refusal([(PANEL << 1) | 1, 0xFE, 0x30], {"rules": []}, 0)

    class _Api:
        adapter = 0

        def _req(self, method, path, body=None):
            return {"rules": OWNER_INPUT_RULES}

    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="0,2,3",
                                               lamps_read_only=False))
    with pytest.raises(LampNotAllowed, match=r"no controller lists the rules"):
        master._check(master._normalize([{"bits": 24, "bytes": _event(PANEL, FREE_INSTANCE)}]))
    master.api = _Api()
    with pytest.raises(LampNotAllowed, match=r"rule\(s\) \['hall'\] could fire"):
        master._check(master._normalize([{"bits": 24, "bytes": _event(PANEL, KEYED_INSTANCE)}]))
    master._check(master._normalize([{"bits": 24, "bytes": _event(PANEL, FREE_INSTANCE)}]))


def test_off_all_drives_only_the_allowlist(monkeypatch):
    monkeypatch.setattr(hil.api.time, "sleep", lambda _s: None)
    client = _client(shorts=(0, 1, 2, 3, 4))
    client.off_all()
    assert [path for _m, path, _b in client.http.sent] == [
        "adapters/0/physical-devices/%d/target-state" % s for s in sorted(LAMPS)]


def test_restore_states_skips_a_lamp_it_may_not_drive(capsys):
    client = _client()
    client.restore_states([{"short_address": OWNER, "state": {"power": "on", "level": 9}}])
    assert client.http.sent == []
    assert "SA1 is outside" in capsys.readouterr().out


def test_the_foreign_master_is_refused_before_it_reaches_the_wb(monkeypatch):
    cfg = dataclasses.replace(load_config(), lamp_shorts="0,2,3", lamps_read_only=False)
    master = ForeignMaster(cfg)
    sent = []
    monkeypatch.setattr(master, "_run_client", lambda frames: sent.append(frames) or [])
    for door in (lambda: master.dapc(OWNER, 100),
                 lambda: master.set_rgb(OWNER, 254, 0, 0),
                 lambda: master.set_cct(OWNER, 2700),
                 lambda: master.goto_scene(12),
                 lambda: master.broadcast_dapc(10),
                 lambda: master.raw16(_short_wire(OWNER, command=True), REMOVE_FROM_SCENE_12)):
        with pytest.raises(LampNotAllowed):
            door()
    assert sent == []
    master.dapc(2, 100)
    master.cmd(OWNER, QUERY_ACTUAL_LEVEL)
    assert [f[0]["bytes"] for f in sent] == [[_short_wire(2), 100],
                                             [_short_wire(OWNER, command=True),
                                              QUERY_ACTUAL_LEVEL]]


def test_the_guard_names_what_a_passed_request_writes(monkeypatch):
    keys = _applying(pending={7}, bindings={7: 2}).check_request("POST", GROUP_APPLY)
    assert keys == [("group_matrix/7", frozenset({"applied"})),
                    ("gear/2", frozenset({"groups"}))]
    assert _guard().check_frame(_short_wire(2), 100) == [
        ("gear/2", frozenset({"*"})), ("shown/2", frozenset({"*"}))]
    assert _guard().check_frame(_short_wire(2, command=True), QUERY_ACTUAL_LEVEL) == []
    assert _guard().check_frame(_short_wire(2, command=True), ADD_TO_GROUP_5) == [
        ("gear/2", frozenset({"*"}))]


def test_the_client_and_the_wb_master_log_what_they_write(monkeypatch):
    client = _client(bindings={8: 2})
    log = write_log.WriteLog("t", client.base)
    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="0,2,3",
                                               lamps_read_only=False))
    monkeypatch.setattr(master, "_run_client", lambda frames: [])
    with write_log.recording(log):
        client.poller.patch({"enabled": False})
        client.vlamps.ts(8, {"power": "on"})
        master.dapc(3, 100)
    assert log.changed("settings/poller", "enabled") and not log.changed("settings/dali")
    assert log.changed("shown/2") and log.changed("gear/3") and not log.changed("shown/0")


def test_a_read_only_run_never_injects_an_input_event(monkeypatch):
    class _Api:
        adapter = 0

        def _req(self, method, path, body=None):
            return {"rules": {"rules": []}}

        def segment_shorts(self):
            return []

    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="",
                                               lamps_read_only=True), api=_Api())
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY.*automations"):
        master._check(master._normalize([{"bits": 24, "bytes": _event(PANEL, FREE_INSTANCE)}]))


RESTARTS = (("POST", "redundancy/switchover", {}),
            ("POST", "firmware/updates", {"url": "http://x/app.bin"}),
            ("PATCH", "settings/dali", {"application_active": True}))
RESTART_RULES = [
    _input_rule("morning", {"kind": "controller_starts"}),
    _input_rule("failover", {"kind": "controller_becomes_active"}),
    _input_rule("hil-vg-10", {"kind": "controller_becomes_active"}),
    _input_rule("asleep", {"kind": "controller_starts"}, enabled=False),
    _input_rule("dusk", {"kind": "at_solar", "event": "sunset", "offset_ms": 0}),
]


def test_a_restart_or_handover_is_refused_while_an_owner_rule_fires_when_a_controller_starts():
    owner = LampGuard(LAMPS, restart_rules=lambda: ["failover", "morning"])
    quiet = LampGuard(LAMPS, restart_rules=lambda: [])
    for method, path, body in RESTARTS + HANDOVERS:
        with pytest.raises(LampNotAllowed, match=r"failover, morning fire when a controller "
                                                 r"starts or becomes active"):
            owner.check_request(method, path, body)
        with pytest.raises(LampNotAllowed, match=r"no controller lists the owner's rules"):
            LampGuard(LAMPS).check_request(method, path, body)
        assert ("shown/*", frozenset({"*"})) in quiet.check_request(method, path, body)
    owner.check_request("PATCH", "settings/dali", {"device_short_address": 3})
    owner.check_request("PATCH", "settings/poller", {"enabled": False})


def test_the_client_asks_the_owner_rules_before_a_handover_and_books_it_as_moving_any_lamp():
    owner = _client(rules=RESTART_RULES)
    log = write_log.WriteLog("t", owner.base)
    with write_log.recording(log):
        with pytest.raises(LampNotAllowed, match=r"settings/redundancy refused: the owner's "
                                                 r"rule\(s\) failover, morning"):
            owner.redundancy.patch_settings({"peer_url": "http://192.0.2.9:81"})
        with pytest.raises(LampNotAllowed, match=r"settings/dali refused: .* failover"):
            owner.dali_settings.patch({"application_active": False})
    assert owner.http.sent == [] and not log.changed("shown/5")
    quiet = _client(rules=RESTART_RULES[2:])
    with write_log.recording(log):
        quiet.redundancy.patch_settings({"peer_url": "http://192.0.2.9:81"})
    assert [path for _m, path, _b in quiet.http.sent] == ["settings/redundancy"]
    assert log.changed("shown/5")


FORGET = "adapters/0/physical-devices/%d"


def test_a_forget_is_a_write_to_its_short_and_never_passes_read_only():
    with pytest.raises(LampNotAllowed, match=r"forget of SA1 refused: SA1 is outside "
                                             r"HIL_LAMP_SHORTS=0,2-3"):
        _guard().check_request("DELETE", FORGET % OWNER)
    with pytest.raises(LampNotAllowed, match=r"forget of SA2 refused: HIL_LAMPS_READ_ONLY=1"):
        _guard(read_only=True).check_request("DELETE", FORGET % 2)
    assert ("device/2", frozenset({"*"})) in _guard().check_request("DELETE", FORGET % 2)
    _guard(read_only=True).check_request("PATCH", FORGET % OWNER, {"notes": "x"})


class _ForgetApi:
    adapter = 0

    def __init__(self, lamp_shorts, read_only, registry=(1, 3), present=(1, 3)):
        self.cfg = HilConfig(lamp_shorts=lamp_shorts, lamps_read_only=read_only,
                             serial_remote="")
        self.guard = LampGuard.for_config(self.cfg)
        self.registry, self.present = list(registry), list(present)

    def lamp_addrs(self):
        return [short for short in self.registry if short in self.cfg.lamp_short_set()]

    def present_addrs(self):
        return list(self.present)


def test_the_forget_test_takes_a_present_lamp_the_guard_lets_it_forget():
    assert test_pd_forget.forget_target(_ForgetApi("3", read_only=False)) == 3
    for api, why in ((_ForgetApi("3", read_only=True), r"forget of SA3 refused: "
                                                         r"HIL_LAMPS_READ_ONLY=1"),
                     (_ForgetApi("3", read_only=False, present=(1,)), r"no present lamp"),
                     (_ForgetApi("", read_only=False), r"HIL_LAMP_SHORTS=\(none\)")):
        with pytest.raises(pytest.skip.Exception, match=why):
            test_pd_forget.forget_target(api)


def test_the_client_refuses_a_switchover_and_a_reboot_before_they_start():
    client = _client(rules=RESTART_RULES)
    assert client.restart_rules() == ["failover", "morning"]
    log = write_log.WriteLog("t", client.base)
    with write_log.recording(log):
        with pytest.raises(LampNotAllowed, match=r"switchover refused: the owner's rule\(s\) "
                                                 r"failover, morning"):
            client.redundancy.switchover()
        with pytest.raises(LampNotAllowed, match=r"a reboot of .* refused"):
            with client.expect_reboot():
                pass
    assert client.http.sent == [] and not log.changed("shown/5")
    quiet = _client(rules=RESTART_RULES[2:])
    quiet.redundancy.switchover()
    with quiet.expect_reboot():
        pass
    assert [path for _m, path, _b in quiet.http.sent] == ["redundancy/switchover"]


def test_a_handover_asks_both_units_before_the_first_switchover():
    quiet = _client()
    stale = _client(rules=RESTART_RULES[1:2])
    pair.check_handovers(quiet, quiet)
    with pytest.raises(LampNotAllowed, match=r"handover .* refused: .* failover"):
        pair.check_handovers(quiet, stale)
    assert quiet.http.sent == [] and stale.http.sent == []


def test_an_unsettled_pair_is_left_as_found_while_an_owner_rule_fires_on_a_start():
    primary, peer = _client(), _client(rules=RESTART_RULES[1:2])
    for client in (primary, peer):
        client.redundancy.get = lambda: {"active": False}
    with pytest.raises(LampNotAllowed, match=r"failover"):
        pair.settle(primary, peer, timeout_s=0.1)
    assert primary.http.sent == [] and peer.http.sent == []



HANDOVERS = (("POST", "redundancy/switchover", {}),
             ("PATCH", "settings/dali", {"application_active": True}),
             ("PATCH", "settings/dali", {"application_active": False}),
             ("PATCH", "settings/redundancy", {"peer_url": "http://192.0.2.9:81"}))


def test_a_read_only_run_never_hands_the_bus_over():
    read_only = LampGuard(LAMPS, read_only=True, restart_rules=lambda: [])
    driving = LampGuard(LAMPS, restart_rules=lambda: [])
    for method, path, body in HANDOVERS:
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY=1 never restarts a "
                                                 r"controller or hands the bus over"):
            read_only.check_request(method, path, body)
        driving.check_request(method, path, body)
    read_only.check_request("PATCH", "settings/dali", {"device_short_address": 3})


def test_a_read_only_run_leaves_an_unsettled_pair_as_it_found_it():
    primary = _client(read_only=True)
    peer = _client(read_only=True)
    for client in (primary, peer):
        client.redundancy.get = lambda: {"active": False}
    with pytest.raises(LampNotAllowed, match=r"never restarts a controller or hands the bus"):
        pair.settle(primary, peer, timeout_s=0.1)
    assert primary.http.sent == [] and peer.http.sent == []


def test_a_read_only_run_restarts_no_controller_and_names_the_go_ahead_it_needs():
    read_only = LampGuard(LAMPS, read_only=True, restart_rules=lambda: [])
    for method, path, body in (("POST", "firmware/updates", {"url": "http://x/app.bin"}),) \
            + HANDOVERS:
        with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY=0, the owner's go-ahead"):
            read_only.check_request(method, path, body)
    client = _client(read_only=True)
    log = write_log.WriteLog("t", client.base)
    with write_log.recording(log):
        with pytest.raises(LampNotAllowed, match=r"a reboot of .* refused: HIL_LAMPS_READ_ONLY=1"):
            with client.expect_reboot():
                pass
    assert client.http.sent == [] and not log.changed("shown/5")


def test_a_segment_command_that_passes_names_every_lamp_it_reaches():
    guard = _guard(segment=(0, 2, 3))
    exact = {("shown/%d" % short, frozenset({"*"})) for short in (0, 2, 3)}
    for method, path in (("PUT", "adapters/0/groups/5/target-state"),
                         ("POST", "adapters/0/scenes/4/recall")):
        assert exact <= set(guard.check_request(method, path, {"power": "on"})), path
    assert exact <= set(guard.check_frame(0xFE, 100))
    assert guard.check_frame(0xFF, QUERY_ACTUAL_LEVEL) == []
