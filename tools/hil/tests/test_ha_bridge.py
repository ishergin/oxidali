import time

import pytest

from hil import mqtt_tap
from hil.wait import wait_until

pytestmark = pytest.mark.ha_bridge

CONNECT_WAIT_S = 20.0
STATE_WAIT_S = 12.0
COLOR_RAMP_S = 3.0


def _vl_state(api, vl_id):
    return api.vlamps.get(vl_id)["state"]


def _cct_lamp(api):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        if (lamp["ha_entity_enabled"] and lamp["capabilities"]["cct"]
                and (lamp.get("binding") or {}).get("physical_short_address") is not None):
            return lamp["virtual_lamp_id"]
    pytest.skip("no exposed CCT-capable bound virtual lamp on this bench")


def _shared_groups_lamp(api):
    matrix = api.groups.matrix()
    for row in matrix.get("rows", []):
        applied = [i for i, v in enumerate(row.get("applied", [])) if v]
        applied = [g for g in applied if g != 1]
        if len(applied) >= 2:
            return row["virtual_lamp_id"], applied[0], applied[1]
    pytest.skip("no lamp with two applied groups (beyond group 1) on this bench")


def _enter_and_connect(guard, api, mqtt_counters):
    guard.enter()
    assert wait_until(lambda: mqtt_counters.get("connected") == 1, CONNECT_WAIT_S), \
        "bridge did not connect to the bench broker"


def _pub(hil_config, topic, payload):
    mqtt_tap.publish(hil_config, topic, payload)


def _cct_command_with_ledger(api, hil_config, topic, payload, vl_id, kelvin):
    _pub(hil_config, topic, payload)
    if wait_until(lambda: _vl_state(api, vl_id)["color_temperature_kelvin"] == kelvin,
                  STATE_WAIT_S):
        return
    api.retries["mqtt_cct_first_command"] += 1
    _pub(hil_config, topic, payload)
    assert wait_until(
        lambda: _vl_state(api, vl_id)["color_temperature_kelvin"] == kelvin,
        STATE_WAIT_S), "CCT command did not land after one counted retry"


def test_session_discovery_and_kelvin_dialect(ha_guard, api, mqtt_counters, hil_config):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    assert wait_until(
        lambda: mqtt_tap.retained(hil_config, ha_guard.topic("availability")) == "online",
        STATE_WAIT_S), "no retained online birth under the test namespace"

    op = api.ha.discovery_publish()
    assert op["status"] == "succeeded", op
    assert op["result"]["entities_published"] > 0, op

    vl_id = _cct_lamp(api)
    config = mqtt_tap.retained_json(
        hil_config, ha_guard.config_topic("light", "a0_vl_%d" % vl_id))
    assert config is not None, "no retained discovery config after a republish"
    assert config.get("color_temp_kelvin") is True, \
        "discovery must declare kelvin or HA reads color_temp as mireds"
    assert "color_temp" in config["supported_color_modes"]
    assert config["unique_id"] == "hiltest_a0_vl_%d" % vl_id
    sources = [entry.get("topic") for entry in config.get("availability") or []]
    assert ha_guard.topic("availability") in sources, (
        "controller availability is not among the entity's sources: %r" % config)
    assert config.get("availability_mode") == "all", (
        "`any` would keep a mains-off fixture available for as long as the "
        "controller lives, which is the defect the second source exists for: %r"
        % config)
    assert "availability_topic" not in config, (
        "both forms sent at once is undefined in HA's schema: %r" % config)


def test_cct_round_trip_and_colour_only_power(ha_guard, api, mqtt_counters,
                                              hil_config, state_snapshot):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    vl_id = _cct_lamp(api)
    set_topic = ha_guard.topic("a0/vl/%d/set" % vl_id)

    _cct_command_with_ledger(
        api, hil_config, set_topic,
        '{"state":"ON","brightness":140,"color_temp":3000}', vl_id, 3000)
    state = _vl_state(api, vl_id)
    assert state["power"] == "on" and state["level"] == 140

    api.vlamps.ts(vl_id, {"color_mode": "cct", "color_temperature_kelvin": 4500})
    assert wait_until(
        lambda: (mqtt_tap.retained_json(
            hil_config, ha_guard.topic("a0/vl/%d/state" % vl_id)) or {}
        ).get("color_temp") == 4500,
        STATE_WAIT_S), "state topic never carried the new kelvin under `color_temp`"
    payload = mqtt_tap.retained_json(hil_config, ha_guard.topic("a0/vl/%d/state" % vl_id))
    assert payload["state"] == "ON", \
        "a colour-only change must not regress a lit lamp to OFF"
    assert payload["brightness"] == 140
    assert "color_temp_kelvin" not in payload, "config flag name is not a payload key"

    _pub(hil_config, set_topic, '{"state":"OFF"}')
    assert wait_until(lambda: _vl_state(api, vl_id)["power"] == "off", STATE_WAIT_S)


def test_group_tile_lights_only_for_the_commanded_group(ha_guard, api, mqtt_counters,
                                                        hil_config, state_snapshot):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    vl_id, g_cmd, g_other = _shared_groups_lamp(api)

    def tile(group_id):
        payload = mqtt_tap.retained_json(
            hil_config, ha_guard.topic("a0/group/%d/state" % group_id))
        return (payload or {}).get("state")

    _pub(hil_config, ha_guard.topic("a0/vl/%d/set" % vl_id),
         '{"state":"ON","brightness":120}')
    assert wait_until(lambda: _vl_state(api, vl_id)["power"] == "on", STATE_WAIT_S)
    assert wait_until(lambda: tile(g_cmd) == "OFF", STATE_WAIT_S), \
        "tile %d lit for an individual lamp command" % g_cmd
    assert tile(g_other) == "OFF"

    _pub(hil_config, ha_guard.topic("a0/group/%d/set" % g_cmd),
         '{"state":"ON","brightness":128}')
    assert wait_until(lambda: tile(g_cmd) == "ON", STATE_WAIT_S), \
        "commanded group %d tile never lit" % g_cmd
    assert tile(g_other) == "OFF", \
        "bystander group %d lit from a shared lamp" % g_other

    _pub(hil_config, ha_guard.topic("a0/group/%d/set" % g_cmd), '{"state":"OFF"}')
    assert wait_until(lambda: tile(g_cmd) == "OFF", STATE_WAIT_S)


@pytest.mark.foreign
def test_foreign_master_group_frame_arms_the_tile(ha_guard, api, mqtt_counters,
                                                  hil_config, state_snapshot, foreign):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    _vl, g_cmd, _g_other = _shared_groups_lamp(api)

    def tile():
        payload = mqtt_tap.retained_json(
            hil_config, ha_guard.topic("a0/group/%d/state" % g_cmd))
        return (payload or {}).get("state")

    foreign.group_dapc(g_cmd, 120)
    assert wait_until(lambda: tile() == "ON", STATE_WAIT_S), \
        "foreign group DAPC never armed the tile"
    foreign.group_dapc(g_cmd, 0)
    assert wait_until(lambda: tile() == "OFF", STATE_WAIT_S), \
        "foreign group DAPC 0 (off) never cleared the tile"


def test_scene_select_recall_and_reset(ha_guard, api, mqtt_counters,
                                       hil_config, state_snapshot):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    vl_id = _cct_lamp(api)

    _pub(hil_config, ha_guard.topic("a0/scene_select/set"), "Scene 0")
    time.sleep(1.0)
    api.vlamps.ts(vl_id, {"color_mode": "cct", "color_temperature_kelvin": 3500})
    assert wait_until(
        lambda: mqtt_tap.retained(
            hil_config, ha_guard.topic("a0/scene_select/state")) == "Scene 0",
        STATE_WAIT_S), "select state never showed the recalled scene"

    api.vlamps.ts(vl_id, {"power": "on", "level": 120})
    assert wait_until(
        lambda: mqtt_tap.retained(
            hil_config, ha_guard.topic("a0/scene_select/state")) == "None",
        STATE_WAIT_S), "select state never reset after a DAPC"


def test_exclusion_retracts_immediately(ha_guard, api, mqtt_counters, hil_config):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    vl_id = _cct_lamp(api)
    api.ha.discovery_publish()
    config_topic = ha_guard.config_topic("light", "a0_vl_%d" % vl_id)
    assert wait_until(lambda: mqtt_tap.retained(hil_config, config_topic) is not None,
                      STATE_WAIT_S)

    try:
        api.vlamps.patch(vl_id, {"ha_entity_enabled": False})
        assert wait_until(
            lambda: mqtt_tap.retained(hil_config, config_topic, timeout_s=2) is None,
            STATE_WAIT_S), "config topic still holds a retracted entity"
    finally:
        api.vlamps.patch(vl_id, {"ha_entity_enabled": True})
    assert wait_until(lambda: mqtt_tap.retained(hil_config, config_topic) is not None,
                      STATE_WAIT_S), "re-enabling never re-announced the config"


def test_counters_move_and_unknown_entity_is_unroutable(ha_guard, api, mqtt_counters,
                                                        hil_config):
    received_before = mqtt_counters.get("commands_received_total")
    unroutable_before = mqtt_counters.get("commands_unroutable_total")
    _enter_and_connect(ha_guard, api, mqtt_counters)

    _pub(hil_config, ha_guard.topic("a0/vl/63/set"), '{"state":"ON"}')
    assert wait_until(
        lambda: mqtt_counters.delta(
            unroutable_before, mqtt_counters.get("commands_unroutable_total")) >= 1,
        STATE_WAIT_S), "unknown entity was not counted unroutable"
    assert mqtt_counters.delta(
        received_before, mqtt_counters.get("commands_received_total")) >= 1
    assert mqtt_counters.get("connects_total") >= 1


def test_disable_says_offline(ha_guard, api, mqtt_counters, hil_config):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    assert wait_until(
        lambda: mqtt_tap.retained(hil_config, ha_guard.topic("availability")) == "online",
        STATE_WAIT_S)
    api.ha.patch({"enabled": False})
    assert wait_until(
        lambda: mqtt_tap.retained(hil_config, ha_guard.topic("availability")) == "offline",
        STATE_WAIT_S), "no retained offline after a deliberate disable"


def test_bench_membership_matches_registry(api):
    matrix = api.groups.matrix()
    lamps = api.vlamps.list()["virtual_lamps"]
    bound = {l["virtual_lamp_id"]: l["binding"]["physical_short_address"]
             for l in lamps if l.get("binding")}
    checked = 0
    for row in matrix.get("rows", []):
        short = bound.get(row["virtual_lamp_id"])
        if short is None or short > 3:
            continue
        pd = api.attributes(short, ["groups"])
        membership = ((pd.get("attributes") or {}).get("groups") or {}).get("membership")
        if not membership:
            continue
        mask = int(membership["value"])
        gear_groups = [g for g in range(16) if mask & (1 << g)]
        applied = [i for i, v in enumerate(row.get("applied", [])) if v]
        assert gear_groups == applied, (
            "vl %d / SA%d: gear reports groups %s, matrix applied says %s"
            % (row["virtual_lamp_id"], short, gear_groups, applied))
        checked += 1
    if checked == 0:
        pytest.skip("no real fixture with a gear-reported membership read")


@pytest.mark.optical
def test_cct_over_mqtt_is_optically_real(ha_guard, api,
                                         mqtt_counters, hil_config,
                                         lamps, camera_oracle, calibration,
                                         state_snapshot, needs_capability):
    label, spread = camera_oracle.cct_fingerprint_spread()
    if label is None:
        pytest.skip("no cct fingerprints in this calibration")
    if spread < 1.15:
        pytest.skip("cct fingerprint spread %.2f < 1.15 — optically degenerate" % spread)
    short = lamps.short(label)
    if not needs_capability.ensure(short, "cct"):
        pytest.skip("cct capability absent on fingerprint lamp %s" % short)
    lamps_all = api.vlamps.list()["virtual_lamps"]
    vl_id = next((l["virtual_lamp_id"] for l in lamps_all
                  if (l.get("binding") or {}).get("physical_short_address") == short
                  and l["ha_entity_enabled"]), None)
    if vl_id is None:
        pytest.skip("fingerprint lamp SA%d is not an exposed virtual lamp" % short)

    _enter_and_connect(ha_guard, api, mqtt_counters)
    set_topic = ha_guard.topic("a0/vl/%d/set" % vl_id)
    ratios = {}
    for kelvin in (2700, 6500):
        payload = '{"state":"ON","brightness":120,"color_temp":%d}' % kelvin
        _cct_command_with_ledger(api, hil_config, set_topic, payload, vl_id, kelvin)
        time.sleep(COLOR_RAMP_S)
        m = camera_oracle.require_colour(
            camera_oracle.measure(label, name="mqtt_cct_%d" % kelvin), label, "cct")
        ratios[kelvin] = m["rb_ratio"]
    _pub(hil_config, set_topic, '{"state":"OFF"}')
    camera_oracle.judge_cct_order(ratios)


def _group_members(api, group_id):
    matrix = api.groups.matrix()
    lamps = {l["virtual_lamp_id"]: l
             for l in api.vlamps.list()["virtual_lamps"]}
    shorts = []
    for row in matrix.get("rows", []):
        applied = row.get("applied") or []
        if group_id >= len(applied) or not applied[group_id]:
            continue
        binding = (lamps.get(row["virtual_lamp_id"], {}).get("binding") or {})
        short = binding.get("physical_short_address")
        if short is not None:
            shorts.append(short)
    return sorted(shorts)


@pytest.mark.hil_id("HIL-MQTT-11")
@pytest.mark.sniffer
def test_a_group_command_reaches_the_lamps_not_only_the_tile(ha_guard, api,
                                                            mqtt_counters,
                                                            hil_config, sniffer,
                                                            state_snapshot,
                                                            test_artifacts):
    _enter_and_connect(ha_guard, api, mqtt_counters)
    _vl, g_cmd, _g_other = _shared_groups_lamp(api)
    members = [s for s in _group_members(api, g_cmd) if s in api.optical_addrs()]
    if not members:
        pytest.skip("group %d has no bound luminaire member on this bench" % g_cmd)

    for short in members:
        api.ts(short, {"power": "on", "level": 60})
    time.sleep(1.0)

    with sniffer.window() as win:
        _pub(hil_config, ha_guard.topic("a0/group/%d/set" % g_cmd),
             '{"state":"ON","brightness":200,"color_temp":2700}')
        win.expect_frame("DAPC group %d -> level" % g_cmd)

    levels = {}
    for short in members:
        assert wait_until(
            lambda s=short: ((api.state(s).get("state") or {}).get("level") or 0) > 60,
            STATE_WAIT_S), \
            "group member %d never moved for an MQTT group command" % short
        levels[short] = (api.state(short).get("state") or {}).get("level")

    activated = {s: api.colour_is_activated(s) for s in members}
    test_artifacts.attach_json("mqtt_group_effect", {
        "group": g_cmd, "members": members, "levels_after": levels,
        "temporary_colour_type_is_mask": activated,
    })

    silent = [s for s, v in activated.items() if v is None]
    assert not silent, \
        "gear did not answer QUERY COLOUR VALUE(208) on %r — no reading, not a verdict" % silent
    stale = [s for s, v in activated.items() if v is False]
    assert not stale, \
        "the colour half of an MQTT group command never activated on %r" % stale

    _pub(hil_config, ha_guard.topic("a0/group/%d/set" % g_cmd), '{"state":"OFF"}')
