import json
import os
import re
import time
from collections import namedtuple

from hil.api import ApiError, CapabilityUnsupported, _HomeAssistantSettings, _PollerSettings
from hil.lamp_guard import TEST_RULE_PREFIX, LampNotAllowed, named, only_hil_rules_appended
from hil.write_log import WriteLog, writes_path

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

SETTING_ROUTES = {"ha": "home-assistant"}
GROUP_FIELDS = ("name", "ha_entity_enabled")
MATRIX_FIELDS = ("desired", "applied")
SCENE_FIELDS = ("name", "ha_select_enabled")
VL_FIELDS = ("name", "binding", "ha_entity_enabled")
RECORD_FIELDS = ("name", "notes", "device_type_source", "device_type_effective",
                 "color_mode_source", "color_mode_effective", "dt8_auto_activation_repair",
                 "dt8_rgbwaf_control_assert")
RECORD_WRITES = {"device_type_source": "device_type_override",
                 "device_type_effective": "device_type_override",
                 "color_mode_source": "color_mode_override",
                 "color_mode_effective": "color_mode_override"}
SESSION_PREFIX = "production_state-"
SESSION_STAMP = re.compile(r"[^0-9T]")
READ_ONLY_REFUSED = ("hcl/", "hcl_override/", "time")
SHOWN_KEY = "shown/"
HELD_OPEN = ("prod_state: %s stays open although it is restored: the newer open session %s "
             "wrote some of what it wrote, so that one's next restore would put a test value "
             "back; it closes in the walk that closes that one")
UNDRIVEN = ("prod_state: putting %s back is a visible action, so this restore only reports "
            "it; rerun `hil state restore` with HIL_LAMPS_READ_ONLY=0 and HIL_LAMP_SHORTS "
            "naming them")
Restoration = namedtuple("Restoration", "residual foreign complete")

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
        "rule_toggles": api.rules_toggles(),
        "hcl": api.hcl.list(),
        "vl": api.vlamps.list_unfiltered(),
        "groups": api.groups.list()["groups"],
        "group_matrix": api.groups.matrix(),
        "scenes_meta": api.scenes.list()["scenes"],
        "scenes": api._scene_snapshot(),
        "policies": api._req("GET", "policies"),
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


def _members(snap):
    shorts_of_vl = {v["virtual_lamp_id"]: (v.get("binding") or {}).get("physical_short_address")
                    for v in (snap.get("vl") or {}).get("virtual_lamps", [])}
    members = {}
    for row in (snap.get("group_matrix") or {}).get("rows", []):
        short = shorts_of_vl.get(row["virtual_lamp_id"])
        for gid, applied in enumerate(row.get("applied") or []):
            if applied and short is not None:
                members.setdefault(gid, set()).add(str(short))
    return members


def _target_shorts(snap, sched, members):
    shorts = set()
    for target in sched.get("targets") or []:
        if target.get("scope") == "broadcast":
            shorts |= set(snap.get("devices") or {})
        else:
            shorts |= set().union(*(members.get(g, set()) for g in target.get("group_ids") or []))
    return shorts


def hcl_owned(snap):
    members, owned = _members(snap), {}
    for sched in snap.get("hcl") or []:
        if not sched.get("enabled"):
            continue
        points = sched.get("points") or []
        fields = ()
        if any(p.get("color_temperature_kelvin") is not None for p in points):
            fields += SHOWN_COLOUR
        if any(p.get("level_mode") in ("absolute", "last_active") for p in points):
            fields += SHOWN_LEVEL
        for short in _target_shorts(snap, sched, members):
            owned.setdefault(short, set()).update(fields)
    return owned


def schedule_shorts(snap):
    members = _members(snap)
    return {s["schedule_id"]: _target_shorts(snap, s, members) for s in snap.get("hcl") or []}


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
    return [line for _key, _field, line in differences(before, after, shown_shorts)]


def differences(before, after, shown_shorts=None):
    out = _diff_settings(before, after) + _diff_overrides(before, after)
    if before.get("timezone") != after.get("timezone"):
        out.append(("time", "timezone", "timezone %r -> %r" % (before.get("timezone"),
                                                             after.get("timezone"))))
    out += _diff_hcl(before, after) + _diff_groups(before, after) + _diff_scenes(before, after)
    if before["rules"].get("source") != after["rules"].get("source"):
        out.append(("rules", None, "rules source differs"))
    out += [("rule/%s" % name, "enabled", line) for name, line in
            toggle_differences(before.get("rule_toggles"), after.get("rule_toggles"))]
    if before["adapter"].get("enabled") != after["adapter"].get("enabled"):
        out.append(("adapter/%s" % before["adapter"].get("adapter_id", 0), "enabled",
                    "adapter enabled %r -> %r" % (before["adapter"].get("enabled"),
                                                 after["adapter"].get("enabled"))))
    out += _diff_vl(before, after) + _diff_devices(before, after, shown_shorts)
    was, now = before.get("policies"), after.get("policies") or {}
    out += [("policies", k, "policies %s %r -> %r" % (k, v, now.get(k)))
            for k, v in sorted((was or {}).items()) if now.get(k) != v]
    return out


def toggle_differences(was, now):
    now = now or {}
    return [(name, "rule %r enabled %r -> %r" % (name, enabled, now.get(name)))
            for name, enabled in sorted((was or {}).items()) if now.get(name) != enabled]


def toggle_residue(was, now):
    return [line for _name, line in toggle_differences(was, now)] if was is not None else []


def _norm(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def _field_changes(key, label, was, now, fields):
    return [(key, f, "%s %s %r -> %r" % (label, f, was.get(f), now.get(f)))
            for f in fields if was.get(f) != now.get(f)]


def _diff_settings(before, after):
    out = []
    for name, fields in list(SETTINGS.items()) + [("ha", RESTORABLE_HA)]:
        was, now = before["settings"][name], after["settings"][name]
        out += [("settings/%s" % SETTING_ROUTES.get(name, name), f,
                 "settings/%s.%s %r -> %r" % (name, f, was.get(f), now.get(f)))
                for f in fields if was.get(f) != now.get(f)]
    return out


def _diff_overrides(before, after):
    was, now = before.get("hcl_overrides"), after.get("hcl_overrides") or {}
    if was is None:
        return []
    return [("hcl_override/%s" % sid, None, "hcl overrides: %s %r -> %r"
             % (sid, was.get(sid, False), now.get(sid, False)))
            for sid in sorted(set(was) | set(now))
            if bool(was.get(sid, False)) != bool(now.get(sid, False))]


def _diff_hcl(before, after):
    was = {s["schedule_id"]: s for s in before.get("hcl") or []}
    now = {s["schedule_id"]: s for s in after.get("hcl") or []}
    out = [("hcl/%s" % sid, None, "hcl %s %s" % (sid, "appeared" if sid in now else "is gone"))
           for sid in sorted(set(was) ^ set(now))]
    for sid in sorted(set(was) & set(now)):
        out += _field_changes("hcl/%s" % sid, "hcl %s" % sid, was[sid], now[sid],
                              sorted(set(was[sid]) | set(now[sid])))
    return out


def _diff_groups(before, after):
    was = {g["group_id"]: g for g in before.get("groups") or []}
    now = {g["group_id"]: g for g in after.get("groups") or []}
    out = []
    for gid in sorted(set(was) | set(now)):
        out += _field_changes("group/%d" % gid, "group %d" % gid, was.get(gid, {}),
                              now.get(gid, {}), GROUP_FIELDS)
    rows = [{r["virtual_lamp_id"]: r for r in (snap.get("group_matrix") or {}).get("rows", [])}
            for snap in (before, after)]
    for vl in sorted(set(rows[0]) | set(rows[1])):
        out += _field_changes("group_matrix/%d" % vl, "group matrix VL%d" % vl,
                              rows[0].get(vl, {}), rows[1].get(vl, {}), MATRIX_FIELDS)
    return out


def _diff_scenes(before, after):
    was = {s["scene_id"]: s for s in before.get("scenes") or []}
    now = {s["scene_id"]: s for s in after.get("scenes") or []}
    out = []
    for sid in sorted(set(was) | set(now)):
        a, b = was.get(sid, {}), now.get(sid, {})
        out += _field_changes("scene/%d" % sid, "scene %d" % sid, a, b, SCENE_FIELDS)
        rows = [{r["virtual_lamp_id"]: r["desired"] for r in x.get("rows") or []} for x in (a, b)]
        out += [("scene_matrix/%d/%d" % (sid, vl), "desired", "scene %d VL%d %r -> %r"
                 % (sid, vl, rows[0].get(vl), rows[1].get(vl)))
                for vl in sorted(set(rows[0]) | set(rows[1]))
                if _norm(rows[0].get(vl)) != _norm(rows[1].get(vl))]
    return out


def _diff_vl(before, after):
    now = {v["virtual_lamp_id"]: v for v in after["vl"]["virtual_lamps"]}
    out = []
    for v in before["vl"]["virtual_lamps"]:
        out += _field_changes("vl/%d" % v["virtual_lamp_id"], "VL%d" % v["virtual_lamp_id"],
                              v, now.get(v["virtual_lamp_id"], {}), VL_FIELDS)
    known = {v["virtual_lamp_id"] for v in before["vl"]["virtual_lamps"]}
    for lid, v in sorted(now.items()):
        if lid not in known:
            out.append(("vl/%d" % lid, None, "VL%d is not in the snapshot%s" % (
                lid, " and is bound %r" % v["binding"] if v.get("binding") else "")))
    return out


def _diff_devices(before, after, shown_shorts):
    owned, out = hcl_owned(before), []
    for short, was in before["devices"].items():
        shown = shown_shorts is None or int(short) in shown_shorts
        out += _diff_device(short, was, after["devices"].get(short), shown,
                            owned.get(short, set()))
    out += [("device/%s" % short, None, "SA%s is new in the registry" % short)
            for short in sorted(set(after["devices"]) - set(before["devices"]), key=int)]
    return out


def _diff_device(short, was, now, shown=True, owned=frozenset()):
    if now is None:
        return [("device/%s" % short, None, "SA%s is gone from the registry" % short)]
    out = [("device/%s" % short, RECORD_WRITES.get(f, f), line) for _k, f, line in
           _field_changes("", "SA%s" % short, was["record"], now["record"], RECORD_FIELDS)]
    out += _field_changes("gear/%s" % short, "SA%s gear config" % short, was["config"],
                          now["config"], sorted(set(was["config"]) | set(now["config"])))
    out += [("gear/%s" % short, key, "SA%s gear %s %r -> %r" % (short, key, was[key], now[key]))
            for key in ("groups", "scenes") if was[key] != now[key]]
    fields = [k for k in SHOWN if k not in owned]
    shown_was = {k: was["state"].get(k) for k in fields}
    shown_now = {k: now["state"].get(k) for k in fields}
    if shown and shown_was != shown_now:
        out.append(("shown/%s" % short, None,
                    "SA%s shows %r, was %r" % (short, shown_now, shown_was)))
    return out


RESTORABLE_HA = _HomeAssistantSettings.RESTORABLE


def last_path(cfg):
    return cfg.state_dir / "production_state_last.json"


def session_path(cfg, taken_at):
    return cfg.state_dir / ("%s%s.json" % (SESSION_PREFIX, SESSION_STAMP.sub("", taken_at)))


def open_sessions(cfg):
    found = [p for p in cfg.state_dir.glob(SESSION_PREFIX + "*.json")
             if not p.name.endswith(".writes.json") and unrestored(p)]
    if unrestored(last_path(cfg)):
        found.append(last_path(cfg))
    return sorted(found, key=lambda p: load(p).get("taken_at") or "", reverse=True)


FULL_RESTORE = "`hil state restore --all`"
READ_ONLY_WHY = ("a read-only %s (HIL_LAMPS_READ_ONLY=1) writes no HCL schedule, override "
                 "or time zone, since each can move a lamp at the next tick; rerun it with "
                 "HIL_LAMPS_READ_ONLY=0 to include them" % FULL_RESTORE)


def _schedule_id(schedule):
    return schedule["schedule_id"]


def restore(api, snap, writes, log=print, drive_lamps=True, lamp_shorts=None):
    driven = set(lamp_shorts or ()) if drive_lamps else set()
    if writes is None:
        log("prod_state: no write log of this snapshot's session, so nothing is written back; "
            "the differences follow, and %s writes the whole snapshot back" % FULL_RESTORE)
    else:
        _restore_steps(api, snap, writes, log, drive_lamps, lamp_shorts)
    after = capture(api, prime=True, log=log)
    if writes is not None:
        try:
            _clear_session_overrides(api, snap, after, log, writes)
            after["hcl_overrides"] = _hcl_overrides(api)
        except Exception as exc:
            log("prod_state: clearing HCL overrides FAILED: %r" % (exc,))
    residual, foreign, undriven = _classify(differences(snap, after), writes, driven)
    for line in foreign:
        log("prod_state: changed during the session, not by the toolkit, left as it is: %s"
            % line)
    for line in residual:
        log("prod_state: NOT RESTORED: %s" % line)
    if undriven:
        log(UNDRIVEN % named(undriven))
    return Restoration(residual, foreign, writes is not None and not residual)


def _classify(found, writes, driven):
    residual, foreign, undriven = [], [], set()
    for key, field, line in found:
        if not _ours(writes, key, field, driven):
            foreign.append(line)
            continue
        residual.append(line)
        if key.startswith(SHOWN_KEY) and _shown_short(key) not in driven:
            undriven.add(_shown_short(key))
    return residual, foreign, undriven


def _shown_short(key):
    return int(key[len(SHOWN_KEY):])


def restore_sessions(cfg, client, paths, everything, log=print):
    refused = READ_ONLY_REFUSED if cfg.lamps_read_only else ()
    if everything and refused:
        log("hil state restore --all: %s" % READ_ONLY_WHY)
    residual, held = [], []
    for path in paths:
        snap = load(path)
        writes = WriteLog.everything(snap.get("taken_at"), client.base, refused) if everything \
            else WriteLog.load(writes_path(path), snap.get("taken_at"))
        done = restore(client, snap, writes, log=log, drive_lamps=not cfg.lamps_read_only,
                       lamp_shorts=cfg.lamp_short_set())
        blocker = _newer_writer(held, writes)
        if done.complete and blocker is None:
            mark_restored(path, snap)
        else:
            if done.complete:
                log(HELD_OPEN % (path.name, blocker.name))
            held.append((path, writes))
        residual += done.residual
    return residual


def _newer_writer(held, writes):
    return next((path for path, newer in held
                 if newer is None or writes is None or newer.overlaps(writes)), None)


def newer_open_sessions(cfg, path):
    taken_at = load(path).get("taken_at") or ""
    return [p for p in open_sessions(cfg)
            if p.resolve() != path.resolve() and (load(p).get("taken_at") or "") > taken_at]


def _ours(writes, key, field, driven):
    if writes is None:
        return False
    if key.startswith(SHOWN_KEY):
        return writes.changed(key, exact=True) or (
            _shown_short(key) in driven and writes.changed(key))
    return writes.changed(key, field)


def _restore_steps(api, snap, writes, log, drive_lamps, lamp_shorts):
    def _restore_shown_permitted(api_, snap_, log_, writes_):
        _restore_shown(api_, snap_, log_, writes_, lamp_shorts)

    steps = (_restore_settings, _restore_policies, _restore_timezone, _restore_adapter,
             _restore_rules, _restore_devices, _restore_vl, _restore_groups,
             _restore_scenes, _restore_hcl, _restore_gear_config,
             _restore_gear_tables) + ((_restore_shown_permitted,) if drive_lamps else ())
    for step in steps:
        try:
            step(api, snap, log, writes)
        except Exception as exc:
            log("prod_state: %s FAILED: %r" % (step.__name__, exc))


def _owner(writes, key):
    return lambda field: writes.changed(key, field)


def _left(log, label, fields):
    for field in fields:
        log("prod_state: %s %s differs from the snapshot and the toolkit did not write it: "
            "left as it is" % (label, field))


def _restore_policies(api, snap, log, writes):
    was = snap.get("policies")
    if was is None:
        return
    restore_policies(api, was, log, owned=_owner(writes, "policies"))


def restore_policies(api, was, log=print, owned=None):
    now = api._req("GET", "policies")
    patch = policy_patch(was, now)
    if owned is not None:
        _left(log, "policies", sorted(k for k in patch if not owned(k)))
        patch = {k: v for k, v in patch.items() if owned(k)}
    if patch:
        log("prod_state: policies back to %r" % patch)
        api._req("PATCH", "policies", patch)


def policy_patch(was, now):
    return {k: v for k, v in was.items() if k != "manages_anything" and now.get(k) != v}


def _clear_session_overrides(api, snap, after, log, writes):
    before = snap.get("hcl_overrides")
    if before is None or writes.refuses("hcl_override/"):
        return
    shorts = schedule_shorts(snap)
    for sid, suspended in (after.get("hcl_overrides") or {}).items():
        if not suspended or before.get(sid, False):
            continue
        if writes.changed("hcl_override/%s" % sid, exact=True) or \
                any(writes.changed("shown/%s" % short, exact=True)
                    for short in shorts.get(sid, ())):
            log("prod_state: resuming HCL schedule %s (suspended by this session)" % sid)
            api.hcl.clear_override(sid)
        else:
            log("prod_state: HCL schedule %s is suspended and the toolkit drove nothing it "
                "targets: left as it is" % sid)


def _patch_if_differs(label, current, wanted, fields, patch, log, owned):
    differs = [f for f in fields if f in wanted and current.get(f) != wanted[f]]
    body = {f: wanted[f] for f in differs if owned(f)}
    _left(log, label, [f for f in differs if f not in body])
    if body:
        log("prod_state: restoring %s %s" % (label, sorted(body)))
        patch(body)


def _restore_settings(api, snap, log, writes):
    now, s = _settings(api), snap["settings"]
    for name, route, fields, patch in (
            ("poller", "poller", SETTINGS["poller"], api.poller.patch),
            ("dali", "dali", SETTINGS["dali"], api.dali_settings.patch),
            ("redundancy", "redundancy", SETTINGS["redundancy"],
             api.redundancy.patch_settings),
            ("ha", "home-assistant", RESTORABLE_HA, api.ha.patch)):
        _patch_if_differs(route, now[name], s[name], fields, patch, log,
                          _owner(writes, "settings/" + route))


def _restore_timezone(api, snap, log, writes):
    was = snap.get("timezone")
    if not was or api.time_get().get("timezone") == was:
        return
    if writes.refuses("time"):
        log("prod_state: the time zone differs and is left: %s" % READ_ONLY_WHY)
        return
    if not writes.changed("time", "timezone"):
        _left(log, "the controller's", ["timezone"])
        return
    log("prod_state: restoring timezone %s" % was)
    api.time_set(timezone=was)


def _restore_adapter(api, snap, log, writes):
    _patch_if_differs("adapter", api.adapter_info(), snap["adapter"], ("enabled", "name"),
                      api.adapter_patch, log, _owner(writes, "adapter/%d" % api.adapter))


CONTINUATIONS_PENDING = "continuations_pending"


def continuations_pending(api):
    value = (api.stats().get("rules") or {}).get(CONTINUATIONS_PENDING)
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def commit_refusal(pending):
    if pending is None:
        return ("the firmware reports no rules.%s, so nothing tells whether a document commit "
                "would drop the owner's delayed actions (after/wait)" % CONTINUATIONS_PENDING)
    if pending:
        return ("%d delayed action(s) of the owner's rules are pending (after/wait), and a "
                "document commit drops them" % pending)
    return None


def switch_off_test_rules(api):
    compiled = api._req("GET", "rules?format=json").get("rules") or {}
    names = sorted(r["name"] for r in compiled.get("rules") or []
                   if r.get("enabled", True) and r["name"].startswith(TEST_RULE_PREFIX))
    for name in names:
        api.rule_enable(name, False)
    return names


def _restore_rules(api, snap, log, writes):
    was, now = snap["rules"].get("source") or "", api.rules_get()
    if (now.get("source") or "") == was:
        return
    if not writes.changed("rules"):
        _left(log, "the rules", ["document"])
        return
    refusal = commit_refusal(continuations_pending(api))
    if not only_hil_rules_appended(was, now.get("source") or ""):
        log("prod_state: the rules document differs from the snapshot by more than test "
            "rules; someone else edited it, so it is left as it is, and its test rules %s "
            "are switched off" % switch_off_test_rules(api))
    elif refusal:
        log("prod_state: the test rules %s stay in the rules document, switched off, "
            "because %s; run `hil state restore` once nothing is pending"
            % (switch_off_test_rules(api), refusal))
    else:
        log("prod_state: taking the test rules out of the rules document")
        before = api.rules_toggles()
        api.rules_replace(was, now["revision"])
        _restore_toggles(api, before, log)


def _restore_toggles(api, was, log):
    now = api.rules_toggles()
    for name, enabled in sorted(was.items()):
        if name in now and now[name] != enabled:
            log("prod_state: rule %r back to %s" % (name, "enabled" if enabled else "disabled"))
            api.rule_enable(name, enabled)


def _restore_devices(api, snap, log, writes):
    for short, was in snap["devices"].items():
        now = api.state(int(short))
        body = {f: was["record"][f] for f in ("name", "notes",
                                              "dt8_auto_activation_repair",
                                              "dt8_rgbwaf_control_assert")
                if f in was["record"] and now.get(f) != was["record"][f]}
        body.update(_override_patch(was["record"], now, "device_type"))
        body.update(_override_patch(was["record"], now, "color_mode"))
        mine = {f: v for f, v in body.items() if writes.changed("device/%s" % short, f)}
        _left(log, "SA%s" % short, sorted(set(body) - set(mine)))
        body = mine
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


def _restore_vl(api, snap, log, writes):
    now = {v["virtual_lamp_id"]: v for v in api.vlamps.list_unfiltered()["virtual_lamps"]}
    want = {v["virtual_lamp_id"]: _bound_short(v) for v in snap["vl"]["virtual_lamps"]}
    for lid, have in sorted(now.items()):
        got = _bound_short(have)
        if got is None or want.get(lid) == got:
            continue
        if not writes.changed("vl/%d" % lid, "binding"):
            _left(log, "VL%d" % lid, ["binding"])
            continue
        log("prod_state: unbinding VL%d from %r" % (lid, got))
        _attempt(log, "unbind VL%d" % lid, lambda lid=lid: api.vlamps.unbind(lid))
    for v in snap["vl"]["virtual_lamps"]:
        lid, have = v["virtual_lamp_id"], now.get(v["virtual_lamp_id"], {})
        if want[lid] is not None and want[lid] != _bound_short(have) and \
                writes.changed("vl/%d" % lid, "binding"):
            log("prod_state: binding VL%d -> %r" % (lid, want[lid]))
            _attempt(log, "bind VL%d" % lid,
                     lambda lid=lid: api.vlamps.bind(lid, want[lid]))
        _attempt(log, "VL%d metadata" % lid, lambda lid=lid, have=have, v=v: _patch_if_differs(
            "VL%d" % lid, have, v, ("name", "ha_entity_enabled"),
            lambda body: api.vlamps.patch(lid, body), log, _owner(writes, "vl/%d" % lid)))


def _restore_groups(api, snap, log, writes):
    now = {g["group_id"]: g for g in api.groups.list()["groups"]}
    for g in snap["groups"]:
        gid = g["group_id"]
        _patch_if_differs("group %d" % gid, now.get(gid, {}), g,
                          ("name", "ha_entity_enabled"),
                          lambda body, gid=gid: api.groups.patch(gid, body), log,
                          _owner(writes, "group/%d" % gid))
    now_rows = {r["virtual_lamp_id"]: r for r in api.groups.matrix().get("rows", [])}
    rows = [r for r in snap["group_matrix"]["rows"]
            if _norm(now_rows.get(r["virtual_lamp_id"])) != _norm(r)]
    mine = [r for r in rows if writes.changed("group_matrix/%d" % r["virtual_lamp_id"])]
    _left(log, "the group matrix", ["row of VL%d" % r["virtual_lamp_id"]
                                    for r in rows if r not in mine])
    if not mine:
        return
    log("prod_state: restoring the group matrix rows of VL%s and applying them"
        % ",".join(str(r["virtual_lamp_id"]) for r in mine))
    api.groups.matrix_patch([{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
                             for r in mine])
    res = api.groups.apply()
    if "operation_id" in res:
        api.wait_op(res)


def _restore_scenes(api, snap, log, writes):
    now = {s["scene_id"]: s for s in api._scene_snapshot()}
    for was in snap["scenes"]:
        sid, have = was["scene_id"], now.get(was["scene_id"], {})
        wanted = {"ha_select_enabled": was.get("ha_select_enabled")}
        if was.get("name"):
            wanted["name"] = was["name"]
        _patch_if_differs("scene %d" % sid, have, wanted, ("name", "ha_select_enabled"),
                          lambda body, sid=sid: api.scenes.patch(sid, body), log,
                          _owner(writes, "scene/%d" % sid))
        _restore_scene_rows(api, sid, was.get("rows") or [], have.get("rows") or [], log,
                            writes)


def _restore_scene_rows(api, sid, was_rows, now_rows, log, writes):
    was = {r["virtual_lamp_id"]: r["desired"] for r in was_rows}
    now = {r["virtual_lamp_id"]: r["desired"] for r in now_rows}
    differ = sorted(vl for vl in set(was) | set(now) if _norm(was.get(vl)) != _norm(now.get(vl)))
    mine = [vl for vl in differ if writes.changed("scene_matrix/%d/%d" % (sid, vl))]
    _left(log, "scene %d" % sid, ["row of VL%d" % vl for vl in differ if vl not in mine])
    if not mine:
        return
    log("prod_state: restoring scene %d rows of VL%s and applying them"
        % (sid, ",".join(str(vl) for vl in mine)))
    api.scenes.matrix_patch(sid, [{"virtual_lamp_id": vl, "desired": _scene_desired(was.get(vl))}
                                  for vl in mine])
    res = api.scenes.apply(sid)
    if "operation_id" in res:
        api.wait_op(res)


def _scene_desired(desired):
    if not desired:
        return {"included": False}
    return {k: v for k, v in desired.items() if k != "waf"}


def _restore_hcl(api, snap, log, writes):
    now = {s["schedule_id"]: s for s in api.hcl.list()}
    wanted = {s["schedule_id"]: s for s in snap["hcl"]}
    if writes.refuses("hcl/"):
        if _norm(sorted(now.values(), key=_schedule_id)) != \
                _norm(sorted(wanted.values(), key=_schedule_id)):
            log("prod_state: the HCL schedules differ and are left: %s" % READ_ONLY_WHY)
        return
    for sid in sorted(set(now) - set(wanted)):
        if writes.changed("hcl/%s" % sid):
            log("prod_state: deleting schedule %s the session created" % sid)
            api.hcl.delete(sid)
        else:
            log("prod_state: schedule %s appeared during the session and the toolkit did not "
                "create it: left as it is" % sid)
    for sid, body in sorted(wanted.items()):
        if sid in now:
            _patch_if_differs("schedule %s" % sid, now[sid], body,
                              [k for k in body if k != "schedule_id"],
                              lambda b, sid=sid: api.hcl.patch(sid, b), log,
                              _owner(writes, "hcl/%s" % sid))
        elif writes.changed("hcl/%s" % sid):
            log("prod_state: re-creating schedule %s" % sid)
            api.hcl.create(body)
        else:
            _left(log, "schedule %s" % sid, ["existence"])


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


def _restore_gear_config(api, snap, log, writes):
    for short, was in snap["devices"].items():
        attrs = api.attributes(int(short)).get("attributes") or {}
        now = _gear_config(attrs)
        differs = {f: v for f, v in was["config"].items() if now.get(f) != v}
        body = {f: v for f, v in differs.items() if writes.changed("gear/%s" % short, f)}
        _left(log, "SA%s gear config" % short, sorted(set(differs) - set(body)))
        if body:
            log("prod_state: restoring SA%s gear config %s" % (short, body))
            try:
                _attempt(log, "SA%s gear config" % short,
                         lambda short=short, body=body: api.wait_op(
                             api.write_attrs(int(short), body)))
            except LampNotAllowed as exc:
                log("prod_state: SA%s gear config left as it is: %s" % (short, exc))


def _restore_gear_tables(api, snap, log, writes):
    for short, was in snap["devices"].items():
        tables = [t for t in ("groups", "scenes") if writes.changed("gear/%s" % short, t)]
        if not tables or (was["groups"] is None and
                          not any(v is not None for v in was["scenes"])):
            continue
        s = int(short)
        try:
            api.attr_read_checked(s, groups="groups,scenes")
        except ApiError as exc:
            log("prod_state: SA%d did not answer its table read (%s)" % (s, exc))
            continue
        attrs = api.attributes(s).get("attributes") or {}
        try:
            if "groups" in tables:
                _repair_groups(api, s, was["groups"], _gear_groups(attrs), log)
            if "scenes" in tables:
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


def _restore_shown(api, snap, log, writes, lamp_shorts=None):
    _prime(api, [int(s) for s in snap["devices"]], log, groups=SHOWN_GROUPS)
    live = {str(d["short_address"]): d.get("state") or {}
            for d in api.devices_unfiltered()["physical_devices"]}
    owned = hcl_owned(snap)
    for short, was in snap["devices"].items():
        state = was["state"]
        if state.get("power") not in ("on", "off"):
            continue
        if lamp_shorts is not None and int(short) not in lamp_shorts or \
                not writes.changed("shown/%s" % short):
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
