import dataclasses
import re

import pytest

import hil.api
from hil.config import HilConfig, load as load_config
from hil.foreign import ForeignMaster
from hil.lamp_guard import LampGuard, LampNotAllowed

LAMPS = frozenset({0, 2, 3})
OWNER = 1
REMOVE_FROM_SCENE_12 = 0x50 + 12
QUERY_ACTUAL_LEVEL = 0xA0
ADD_TO_GROUP_5 = 0x60 + 5


def _short_wire(short, command=False):
    return (short << 1) | (1 if command else 0)


def _guard(segment=(0, 2, 3), read_only=False):
    return LampGuard(LAMPS, read_only=read_only, segment=lambda: list(segment))


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
    guard.check_frame(_short_wire(OWNER, command=True), SELECT_DIMMING_CURVE)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    guard.check_frame(_short_wire(2, command=True), STORE_GEAR_FEATURES)
    guard.check_frame(ENABLE_DEVICE_TYPE, 8)
    with pytest.raises(LampNotAllowed, match=r"HIL_LAMPS_READ_ONLY"):
        guard.check_frame(_short_wire(2, command=True), 0xE2)


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
    def __init__(self, shorts, bindings=None):
        self.shorts = shorts
        self.bindings = bindings or {}
        self.sent = []

    def request(self, method, url, json=None, timeout=None):
        path = url.split("/api/v1/", 1)[1]
        if method != "GET":
            self.sent.append((method, path, json))
            return _Response({})
        if path.endswith("/physical-devices"):
            return _Response({"physical_devices": [
                {"short_address": s, "present": True} for s in self.shorts]})
        lamp = re.search(r"virtual-lamps/(\d+)$", path)
        if lamp:
            return _Response({"binding": {
                "physical_short_address": self.bindings.get(int(lamp.group(1)))}})
        return _Response({})

    def close(self):
        pass


def _client(shorts=(0, 1, 2, 3), bindings=None, read_only=False):
    cfg = dataclasses.replace(load_config(), lamp_shorts="0,2,3", gear_shorts="",
                              lamps_read_only=read_only)
    client = hil.api.Client(cfg)
    client.http = _Session(list(shorts), bindings)
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
    )
    for door in doors:
        with pytest.raises(LampNotAllowed):
            door()
    assert client.http.sent == []
    client.vlamps.ts(8, {"power": "on"})
    client.cmd(OWNER, QUERY_ACTUAL_LEVEL)
    assert [path for _m, path, _b in client.http.sent] == [
        "adapters/0/virtual-lamps/8/target-state", "dali/command"]


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
