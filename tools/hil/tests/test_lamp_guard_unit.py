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
    with pytest.raises(LampNotAllowed, match=r"SA1 is outside HIL_LAMP_SHORTS=0,2,3"):
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
