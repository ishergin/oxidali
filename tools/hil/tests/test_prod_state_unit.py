import copy

from hil import prod_state


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
