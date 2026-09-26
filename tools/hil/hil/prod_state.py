import json
import os
import time

from hil.api import ApiError, CapabilityUnsupported, _HomeAssistantSettings, _PollerSettings
from hil.lamp_guard import LampNotAllowed

PRIME_GROUPS = "runtime_status,common_102,dt8_color,dt6_led,groups,scenes,extended"

SHOWN_GROUPS = "runtime_status,dt8_color"

GEAR_CONFIG = {
    ("common_102", "fade_time_ms"): "fade_time_ms",
    ("common_102", "fade_rate"): "fade_rate",
    ("common_102", "power_on_level"): "power_on_level",
    ("common_102", "system_failure_level"): "system_failure_level",
    ("common_102", "min_level"): "min_level",
    ("common_102", "max_level"): "max_level",
    ("extended", "fade_time_ms"): "extended_fade_time_ms",
    ("dt6_led", "dimming_curve"): "dimming_curve",
}

SHOWN = ("power", "level", "color_mode", "color_temperature_kelvin", "rgb")
SHOWN_COLOUR = ("color_mode", "color_temperature_kelvin", "rgb")
SHOWN_LEVEL = ("power", "level")

SETTINGS = {
    "poller": _PollerSettings.RESTORABLE,
    "dali": ("application_active", "device_short_address",
             "dt8_auto_activation_repair", "dt8_rgbwaf_control_assert"),
    "redundancy": ("boot_listen_ms", "enabled", "peer_device_short_address",
                   "peer_url", "probe_interval_ms", "role", "takeover_after_missed"),
}

DTR0 = 0xA3
QUERY_CONTENT_DTR0 = 0x98
SET_SCENE, REMOVE_FROM_SCENE = 0x40, 0x50
QUERY_SCENE_LEVEL = 0xB0
SCENE_REPAIR_ATTEMPTS = 3
ADD_TO_GROUP, REMOVE_FROM_GROUP = 0x60, 0x70
MASK = 255
FRAME_PACE_S = 0.1
FAST_FADE = {"fade_time_ms": 0}
GUARD_OFF = ("0", "false", "no")


def guard_enabled():
    return os.environ.get("HIL_STATE_GUARD", "1") not in GUARD_OFF


def capture(api, prime=True, log=print):
    shorts = [d["short_address"] for d in api.devices_unfiltered()["physical_devices"]]
    overrides = _hcl_overrides(api)
    if prime:
        _prime(api, shorts, log)
    return {
        "hcl_overrides": overrides,
        "taken_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "health": api.health(),
        "devices": {str(s): _device(api, s) for s in shorts},
        "settings": _settings(api),
        "timezone": api.time_get().get("timezone"),
        "adapter": api.adapter_info(),
        "rules": api.rules_get(),
        "hcl": api.hcl.list(),
        "vl": api.vlamps.list(),
        "groups": api.groups.list()["groups"],
        "group_matrix": api.groups.matrix(),
        "scenes_meta": api.scenes.list()["scenes"],
        "scenes": api._scene_snapshot(),
    }


def _prime(api, shorts, log, groups=PRIME_GROUPS):
    for short in shorts:
        try:
            api.attr_read_checked(short, groups=groups)
        except ApiError as exc:
            log("prod_state: SA%d did not answer its priming read (%s) — its "
                "shown state stays as the registry has it" % (short, exc))


def _device(api, short):
    dev = api.state(short)
    attrs = api.attributes(short).get("attributes") or {}
    return {"record": dev, "state": dev.get("state") or {},
            "config": _gear_config(attrs), "groups": _gear_groups(attrs),
            "scenes": _gear_scenes(attrs)}


def _value(attrs, section, key):
    return ((attrs.get(section) or {}).get(key) or {}).get("value")


def _gear_config(attrs):
    out = {}
    for (section, key), field in GEAR_CONFIG.items():
        value = _value(attrs, section, key)
        if value is not None:
            out[field] = value
    return out


def _gear_groups(attrs):
    return _value(attrs, "groups", "membership")


def _gear_scenes(attrs):
    return [_value(attrs, "scenes", "scene_%d" % n) for n in range(16)]


def _hcl_overrides(api):
    return {s["schedule_id"]: bool(api.hcl.override(s["schedule_id"]).get("suspended"))
            for s in api.hcl.list()}


def hcl_owned(snap):
    shorts_of_vl = {v["virtual_lamp_id"]: (v.get("binding") or {}).get("physical_short_address")
                    for v in (snap.get("vl") or {}).get("virtual_lamps", [])}
    members = {}
    for row in (snap.get("group_matrix") or {}).get("rows", []):
        short = shorts_of_vl.get(row["virtual_lamp_id"])
        for gid, applied in enumerate(row.get("applied") or []):
            if applied and short is not None:
                members.setdefault(gid, set()).add(str(short))
    owned = {}
    for sched in snap.get("hcl") or []:
        if not sched.get("enabled"):
            continue
        points = sched.get("points") or []
        fields = ()
        if any(p.get("color_temperature_kelvin") is not None for p in points):
            fields += SHOWN_COLOUR
        if any(p.get("level_mode") in ("absolute", "last_active") for p in points):
            fields += SHOWN_LEVEL
        for target in sched.get("targets") or []:
            if target.get("scope") == "broadcast":
                shorts = set(snap.get("devices") or {})
            else:
                shorts = set().union(*(members.get(g, set()) for g in target.get("group_ids") or []))
            for short in shorts:
                owned.setdefault(short, set()).update(fields)
    return owned


def _settings(api):
    return {"poller": api.poller.get(), "dali": api.dali_settings.get(),
            "redundancy": api.redundancy.settings(), "ha": api.ha.get()}


def save(snap, path):
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(snap, indent=1, ensure_ascii=False, sort_keys=True))
    os.replace(tmp, path)


def unrestored(path):
    try:
        return bool(load(path).get("session_open"))
    except (OSError, ValueError):
        return False


def mark_restored(path, snap):
    try:
        held = load(path)
    except (OSError, ValueError):
        return
    if held.get("taken_at") == snap.get("taken_at") and held.get("session_open"):
        held["session_open"] = False
        save(held, path)


def load(path):
    return json.loads(path.read_text())


def diff(before, after, shown_shorts=None):
    out = []
    out += _diff_settings(before, after)
    if before.get("hcl_overrides") is not None and \
            before["hcl_overrides"] != after.get("hcl_overrides"):
        out.append("hcl overrides %r -> %r" % (before["hcl_overrides"],
                                               after.get("hcl_overrides")))
    for key in ("timezone", "hcl", "groups", "group_matrix", "scenes"):
        if _norm(before.get(key)) != _norm(after.get(key)):
            out.append("%s differs" % key)
    if before["rules"].get("source") != after["rules"].get("source"):
        out.append("rules source differs")
    if before["adapter"].get("enabled") != after["adapter"].get("enabled"):
        out.append("adapter enabled %r -> %r" % (before["adapter"].get("enabled"),
                                                after["adapter"].get("enabled")))
    out += _diff_vl(before, after)
    owned = hcl_owned(before)
    for short, was in before["devices"].items():
        shown = shown_shorts is None or int(short) in shown_shorts
        out += _diff_device(short, was, after["devices"].get(short), shown,
                            owned.get(short, set()))
    return out


def _norm(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def _diff_settings(before, after):
    out = []
    for name, fields in list(SETTINGS.items()) + [("ha", RESTORABLE_HA)]:
        was, now = before["settings"][name], after["settings"][name]
        for field in fields:
            if was.get(field) != now.get(field):
                out.append("settings/%s.%s %r -> %r" % (name, field, was.get(field),
                                                         now.get(field)))
    return out


def _diff_vl(before, after):
    now = {v["virtual_lamp_id"]: v for v in after["vl"]["virtual_lamps"]}
    out = []
    for v in before["vl"]["virtual_lamps"]:
        other = now.get(v["virtual_lamp_id"], {})
        for field in ("name", "binding", "ha_entity_enabled"):
            if v.get(field) != other.get(field):
                out.append("VL%d %s %r -> %r" % (v["virtual_lamp_id"], field,
                                                  v.get(field), other.get(field)))
    known = {v["virtual_lamp_id"] for v in before["vl"]["virtual_lamps"]}
    for lid, v in sorted(now.items()):
        if lid not in known and v.get("binding"):
            out.append("VL%d (not in the snapshot) bound %r" % (lid, v["binding"]))
    return out


def _diff_device(short, was, now, shown=True, owned=frozenset()):
    if now is None:
        return ["SA%s is gone from the registry" % short]
    out = []
    for field in ("name", "notes", "device_type_source", "device_type_effective",
                  "color_mode_source", "color_mode_effective",
                  "dt8_auto_activation_repair", "dt8_rgbwaf_control_assert"):
        if was["record"].get(field) != now["record"].get(field):
            out.append("SA%s %s %r -> %r" % (short, field, was["record"].get(field),
                                             now["record"].get(field)))
    for key in ("config", "groups", "scenes"):
        if was[key] != now[key]:
            out.append("SA%s gear %s %r -> %r" % (short, key, was[key], now[key]))
    fields = [k for k in SHOWN if k not in owned]
    shown_was = {k: was["state"].get(k) for k in fields}
    shown_now = {k: now["state"].get(k) for k in fields}
    if shown and shown_was != shown_now:
        out.append("SA%s shows %r, was %r" % (short, shown_now, shown_was))
    return out


RESTORABLE_HA = _HomeAssistantSettings.RESTORABLE


def last_path(cfg):
    return cfg.state_dir / "production_state_last.json"


def restore(api, snap, log=print, drive_lamps=True, lamp_shorts=None):
    driven = lamp_shorts if drive_lamps else set()

    def _restore_shown_permitted(api_, snap_, log_):
        _restore_shown(api_, snap_, log_, driven)

    steps = (_restore_settings, _restore_timezone, _restore_adapter,
             _restore_rules, _restore_devices, _restore_vl, _restore_groups,
             _restore_scenes, _restore_hcl, _restore_gear_config,
             _restore_gear_tables) + ((_restore_shown_permitted,) if drive_lamps else ())
    for step in steps:
        try:
            step(api, snap, log)
        except Exception as exc:
            log("prod_state: %s FAILED: %r" % (step.__name__, exc))
    after = capture(api, prime=True, log=log)
    try:
        _clear_session_overrides(api, snap, after, log)
        after["hcl_overrides"] = _hcl_overrides(api)
    except Exception as exc:
        log("prod_state: clearing HCL overrides FAILED: %r" % (exc,))
    residual = diff(snap, after, shown_shorts=driven)
    for line in diff(snap, after):
        if line not in residual:
            log("prod_state: reported, not ours to undo: %s" % line)
    for line in residual:
        log("prod_state: NOT RESTORED: %s" % line)
    return residual


def _clear_session_overrides(api, snap, after, log):
    before = snap.get("hcl_overrides")
    if before is None:
        return
    for sid, suspended in (after.get("hcl_overrides") or {}).items():
        if suspended and not before.get(sid, False):
            log("prod_state: resuming HCL schedule %s (suspended by this session)" % sid)
            api.hcl.clear_override(sid)


def _patch_if_differs(label, current, wanted, fields, patch, log):
    body = {f: wanted[f] for f in fields if f in wanted and current.get(f) != wanted[f]}
    if body:
        log("prod_state: restoring %s %s" % (label, sorted(body)))
        patch(body)


def _restore_settings(api, snap, log):
    now = _settings(api)
    s = snap["settings"]
    _patch_if_differs("poller", now["poller"], s["poller"], SETTINGS["poller"],
                      api.poller.patch, log)
    _patch_if_differs("dali", now["dali"], s["dali"], SETTINGS["dali"],
                      api.dali_settings.patch, log)
    _patch_if_differs("redundancy", now["redundancy"], s["redundancy"],
                      SETTINGS["redundancy"], api.redundancy.patch_settings, log)
    _patch_if_differs("home-assistant", now["ha"], s["ha"], RESTORABLE_HA,
                      api.ha.patch, log)


def _restore_timezone(api, snap, log):
    if snap.get("timezone") and api.time_get().get("timezone") != snap["timezone"]:
        log("prod_state: restoring timezone %s" % snap["timezone"])
        api.time_set(timezone=snap["timezone"])


def _restore_adapter(api, snap, log):
    _patch_if_differs("adapter", api.adapter_info(), snap["adapter"],
                      ("enabled", "name"), api.adapter_patch, log)


def _restore_rules(api, snap, log):
    now = api.rules_get()
    if now.get("source") != snap["rules"].get("source"):
        log("prod_state: restoring the rules document")
        api.rules_replace(snap["rules"]["source"], now["revision"])


def _restore_devices(api, snap, log):
    for short, was in snap["devices"].items():
        now = api.state(int(short))
        body = {f: was["record"][f] for f in ("name", "notes",
                                              "dt8_auto_activation_repair",
                                              "dt8_rgbwaf_control_assert")
                if f in was["record"] and now.get(f) != was["record"][f]}
        body.update(_override_patch(was["record"], now, "device_type"))
        body.update(_override_patch(was["record"], now, "color_mode"))
        if body:
            log("prod_state: restoring SA%s %s" % (short, sorted(body)))
            api.device_patch(int(short), body)


def _override_patch(was, now, kind):
    src, eff = kind + "_source", kind + "_effective"
    if (was.get(src), was.get(eff)) == (now.get(src), now.get(eff)):
        return {}
    return {kind + "_override": was.get(eff) if was.get(src) == "manual_override" else None}


def _bound_short(lamp):
    return (lamp.get("binding") or {}).get("physical_short_address")


def _attempt(log, label, fn):
    try:
        fn()
    except ApiError as exc:
        log("prod_state: %s FAILED: %s" % (label, exc))


def _restore_vl(api, snap, log):
    now = {v["virtual_lamp_id"]: v for v in api.vlamps.list()["virtual_lamps"]}
    want = {v["virtual_lamp_id"]: _bound_short(v) for v in snap["vl"]["virtual_lamps"]}
    for lid, have in sorted(now.items()):
        got = _bound_short(have)
        if got is not None and want.get(lid) != got:
            log("prod_state: unbinding VL%d from %r" % (lid, got))
            _attempt(log, "unbind VL%d" % lid, lambda lid=lid: api.vlamps.unbind(lid))
    for v in snap["vl"]["virtual_lamps"]:
        lid, have = v["virtual_lamp_id"], now.get(v["virtual_lamp_id"], {})
        if want[lid] is not None and want[lid] != _bound_short(have):
            log("prod_state: binding VL%d -> %r" % (lid, want[lid]))
            _attempt(log, "bind VL%d" % lid,
                     lambda lid=lid: api.vlamps.bind(lid, want[lid]))
        _attempt(log, "VL%d metadata" % lid, lambda lid=lid, have=have, v=v: _patch_if_differs(
            "VL%d" % lid, have, v, ("name", "ha_entity_enabled"),
            lambda body: api.vlamps.patch(lid, body), log))


def _restore_groups(api, snap, log):
    now = {g["group_id"]: g for g in api.groups.list()["groups"]}
    for g in snap["groups"]:
        gid = g["group_id"]
        _patch_if_differs("group %d" % gid, now.get(gid, {}), g,
                          ("name", "ha_entity_enabled"),
                          lambda body, gid=gid: api.groups.patch(gid, body), log)
    matrix = api.groups.matrix()
    if _norm(matrix) == _norm(snap["group_matrix"]):
        return
    log("prod_state: restoring the group matrix and applying it")
    rows = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
            for r in snap["group_matrix"]["rows"]]
    api.groups.matrix_patch(rows)
    res = api.groups.apply()
    if "operation_id" in res:
        api.wait_op(res)


def _restore_scenes(api, snap, log):
    if _norm(api._scene_snapshot()) == _norm(snap["scenes"]):
        return
    log("prod_state: restoring scene metadata and rows")
    for scene in api._scene_snapshot():
        extra = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": {"included": False}}
                 for r in scene["rows"]]
        if extra:
            api.scenes.matrix_patch(scene["scene_id"], extra)
    api._scene_restore(snap["scenes"])
    for scene in snap["scenes"]:
        if not scene["rows"]:
            res = api.scenes.apply(scene["scene_id"])
            if "operation_id" in res:
                api.wait_op(res)


def _restore_hcl(api, snap, log):
    now = {s["schedule_id"]: s for s in api.hcl.list()}
    wanted = {s["schedule_id"]: s for s in snap["hcl"]}
    for sid in set(now) - set(wanted):
        log("prod_state: deleting schedule %s the session created" % sid)
        api.hcl.delete(sid)
    for sid, body in wanted.items():
        if sid not in now:
            log("prod_state: re-creating schedule %s" % sid)
            api.hcl.create(body)
        elif _norm(now[sid]) != _norm(body):
            log("prod_state: restoring schedule %s" % sid)
            api.hcl.patch(sid, {k: v for k, v in body.items() if k != "schedule_id"})


def fast_fade(api, shorts, log=print):
    done, failed = [], []
    for short in shorts:
        try:
            view = api.wait_op(api.write_attrs(short, dict(FAST_FADE)))
            (done if view.get("status") == "succeeded" else failed).append(short)
        except (ApiError, OSError) as exc:
            failed.append(short)
            log("prod_state: --fast-fade write failed for SA%02d: %s" % (short, exc))
    log("prod_state: --fast-fade set fade_time_ms=0 (instant) on %d/%d gear, restored "
        "at the end of the session%s"
        % (len(done), len(shorts), "" if not failed else "; failed: %s" % failed))
    return done, failed


def _restore_gear_config(api, snap, log):
    for short, was in snap["devices"].items():
        attrs = api.attributes(int(short)).get("attributes") or {}
        now = _gear_config(attrs)
        body = {f: v for f, v in was["config"].items() if now.get(f) != v}
        if body:
            log("prod_state: restoring SA%s gear config %s" % (short, body))
            _attempt(log, "SA%s gear config" % short,
                     lambda short=short, body=body: api.wait_op(
                         api.write_attrs(int(short), body)))


def _restore_gear_tables(api, snap, log):
    for short, was in snap["devices"].items():
        if was["groups"] is None and not any(v is not None for v in was["scenes"]):
            continue
        s = int(short)
        try:
            api.attr_read_checked(s, groups="groups,scenes")
        except ApiError as exc:
            log("prod_state: SA%d did not answer its table read (%s)" % (s, exc))
            continue
        attrs = api.attributes(s).get("attributes") or {}
        try:
            _repair_groups(api, s, was["groups"], _gear_groups(attrs), log)
            _repair_scenes(api, s, was["scenes"], _gear_scenes(attrs), log)
        except LampNotAllowed as exc:
            log("prod_state: SA%d gear tables left as they are: %s" % (s, exc))


def _repair_groups(api, short, want, have, log):
    if want is None or have is None or want == have:
        return
    log("prod_state: SA%d gear groups 0x%04x -> 0x%04x" % (short, have, want))
    for g in range(16):
        bit = 1 << g
        if (want ^ have) & bit:
            op = (ADD_TO_GROUP if want & bit else REMOVE_FROM_GROUP) + g
            api.cmd(short, op, repeat=2)
            time.sleep(FRAME_PACE_S)


def _repair_scenes(api, short, want, have, log):
    for n, (w, h) in enumerate(zip(want, have)):
        if w is None or h is None or w == h:
            continue
        log("prod_state: SA%d gear scene %d %r -> %r" % (short, n, h, w))
        if w == MASK:
            api.cmd(short, REMOVE_FROM_SCENE + n, repeat=2)
        elif not _set_scene_verified(api, short, n, w):
            log("prod_state: SA%d scene %d did not take %d" % (short, n, w))
        time.sleep(FRAME_PACE_S)


def _set_scene_verified(api, short, n, level):
    for _ in range(SCENE_REPAIR_ATTEMPTS):
        if arm_dtr0(api, short, level):
            api.cmd(short, SET_SCENE + n, repeat=2)
            answer = api.raw((((short << 1) | 1) << 8) | (QUERY_SCENE_LEVEL + n),
                             expects_backward=True)
            if answer.get("success") and answer.get("backward_frame") == level:
                return True
        time.sleep(FRAME_PACE_S)
    return False


def arm_dtr0(api, short, value, attempts=3):
    for _ in range(attempts):
        api.raw((DTR0 << 8) | value)
        answer = api.raw((((short << 1) | 1) << 8) | QUERY_CONTENT_DTR0,
                         expects_backward=True)
        if answer.get("success") and answer.get("backward_frame") == value:
            return True
        time.sleep(FRAME_PACE_S)
    return False


def _restore_shown(api, snap, log, lamp_shorts=None):
    _prime(api, [int(s) for s in snap["devices"]], log, groups=SHOWN_GROUPS)
    live = {str(d["short_address"]): d.get("state") or {}
            for d in api.devices_unfiltered()["physical_devices"]}
    owned = hcl_owned(snap)
    for short, was in snap["devices"].items():
        state = was["state"]
        if state.get("power") not in ("on", "off"):
            continue
        if lamp_shorts is not None and int(short) not in lamp_shorts:
            continue
        mine = [k for k in SHOWN if k not in owned.get(short, set())]
        if "power" not in mine or all(live.get(short, {}).get(k) == state.get(k) for k in mine):
            continue
        log("prod_state: SA%s back to %s" % (short, {k: state.get(k) for k in mine}))
        try:
            api.ts(int(short), _setpoint(state, colour="color_mode" in mine))
        except (ApiError, CapabilityUnsupported, LampNotAllowed) as exc:
            log("prod_state: SA%s target-state refused: %s" % (short, exc))
        time.sleep(FRAME_PACE_S)


def _setpoint(state, colour=True):
    body = {"power": state["power"]}
    if state["power"] == "on" and state.get("level") is not None:
        body["level"] = state["level"]
    if not colour:
        return body
    if state.get("color_mode") == "cct" and state.get("color_temperature_kelvin"):
        body.update(color_mode="cct",
                    color_temperature_kelvin=state["color_temperature_kelvin"])
    elif state.get("color_mode") == "rgb" and state.get("rgb"):
        body.update(color_mode="rgb", rgb=state["rgb"])
    return body
