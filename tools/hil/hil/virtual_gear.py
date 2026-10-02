import json
import os
import re
import subprocess
import time
from dataclasses import dataclass
from functools import partial
from pathlib import Path

from hil import durable, remote_serial, role, serialmon
from hil.gearsim import GearSim, GearSimUnavailable
from hil.lamp_guard import RESERVE_FLOOR, emulated_gtin, spell
from hil.wait import settled

LEDGER = role.LEDGER

SHORT_COUNT = 64
GROUP_COUNT = 16
VL_ID_LIMIT = 64
DEFAULT_PARK = (4, 4, 4)
PARK_KINDS = ("dt6", "cct", "rgb")
LADDER_FIRST = 4
LADDER_COUNTERS = ("sent", "late", "expired")

GROUP_ADDRESS_BASE = 0x80
SELECTOR_BIT = 0x01
QUERY_STATUS = 0x90
QUERY_CONTROL_GEAR_PRESENT = 0x91
YES = 0xFF
PROOF_PROBES = 3
PROOF_ATTEMPTS = 3
CONTENTION_COUNTERS = ("collision_restarts_total", "foreign_frames_total",
                       "backward_early_rejected_total", "backward_late_rejected_total")
WIRE_CONTENTION = ("collisions", "foreign_in_window", "bus_acquire_timeout", "exchange_retries",
                   "retry_exhausted")
ANSWER_DAMAGE = ("backward_undecodable_total", "backward_frame_size_total",
                 "backward_incomplete_total", "backward_multi_answer_total")
WIRE_DAMAGE = ("corrupted_in_window",)
FRAMES_SENT = "frames_sent_by_priority"
QUIET_WIRE_S = 3.0
QUIET_WAIT_MAX_S = 30.0
QUIET_POLL_S = 0.5
EXIT_SETUP, EXIT_SAFETY, EXIT_PEER_RETURNED, EXIT_BLIND = 3, 4, 5, 6

SCAN_MODE = "scan_known_short_addresses"
IDENTITY_GROUPS = "runtime_status"
IDENTITY_BANKS = "identity"
VIRTUAL_ENV = "HIL_VIRTUAL_GEAR"
PARK_VL_NAME = "virtual gear SA%d"
WB_CONFIG = "/etc/wb-mqtt-dali.conf"
SSH_TIMEOUT_S = 30
RETAINED_WAIT_S = 3

RULE_GROUP = re.compile(r"\bgroup\(\s*([0-9]+)\s*[,)]")
RULE_GROUP_NAME = re.compile(r"\bgroup\(\s*\"([^\"]*)\"")
RULE_LAMP = re.compile(r"\blamp\(\s*\"([^\"]*)\"")
RULE_LAMP_ID = re.compile(r"\blamp\(\s*([0-9]+)\s*[,)]")
RULE_DEVICE = re.compile(r"(?<![\w.])device\(\s*([0-9]+)\s*[,)]")
LAMP_SCOPE, GROUP_SCOPE = "virtual_lamp", "group"


class VirtualGearError(RuntimeError):
    pass


class Ledger:
    def __init__(self, path: Path):
        self.path = Path(path)
        self.data = json.loads(self.path.read_text()) if self.path.exists() else {}

    @classmethod
    def of(cls, cfg):
        return cls(Path(cfg.state_dir) / LEDGER)

    def exists(self) -> bool:
        return self.path.exists()

    def update(self, **fields):
        self.data.update(fields)
        durable.write_json(self.path, self.data)

    def append(self, key, value):
        self.update(**{key: list(self.data.get(key, [])) + [value]})

    def remove(self):
        self.path.unlink(missing_ok=True)


def reserve(registry, wb, owner) -> frozenset:
    return frozenset(range(RESERVE_FLOOR)) | frozenset(registry) | frozenset(wb) | frozenset(owner)


def park_shorts(reserved, count) -> list:
    free = [s for s in range(SHORT_COUNT) if s not in reserved]
    if len(free) < count:
        raise VirtualGearError("%d gear asked, %d addresses are free of the reserve"
                               % (count, len(free)))
    return free[:count]


def parse_park(spec) -> tuple:
    parts = [part.strip() for part in spec.split(",")]
    if len(parts) != len(PARK_KINDS) or not all(part.isdigit() for part in parts):
        raise VirtualGearError("a park shape is the %s counts, as in %s; got %r"
                               % ("/".join(PARK_KINDS), ",".join(map(str, DEFAULT_PARK)), spec))
    shape = tuple(int(part) for part in parts)
    if not sum(shape):
        raise VirtualGearError("a park of no gear leaves the tier nothing to drive: %r" % spec)
    return shape


def run_enabled() -> bool:
    return os.environ.get(VIRTUAL_ENV) == "1"


def park_of_kind(park, shape, kind) -> list:
    index = PARK_KINDS.index(kind)
    start = sum(shape[:index])
    return list(park[start:start + shape[index]])


def registry_shorts(api) -> set:
    return {d["short_address"] for d in api.devices_unfiltered()["physical_devices"]}


def vl_ids(api) -> list:
    return sorted(v["virtual_lamp_id"] for v in api.vlamps.list_unfiltered()["virtual_lamps"])


def wb_shorts(cfg) -> set:
    out = subprocess.run(["ssh", *remote_serial.SSH_OPTS, cfg.wb_ssh, "cat %s" % WB_CONFIG],
                         capture_output=True, text=True, timeout=SSH_TIMEOUT_S)
    if out.returncode != 0:
        raise VirtualGearError("could not read %s on %s: %s"
                               % (WB_CONFIG, cfg.wb_ssh, out.stderr.strip()))
    shorts = wb_shorts_of(json.loads(out.stdout), cfg.wb_device, cfg.wb_bus)
    if not shorts:
        raise VirtualGearError("%s lists no gear for device %r bus %d: check HIL_WB_DEVICE and "
                               "HIL_WB_BUS, or the reserve misses every lamp only the WB "
                               "master knows" % (WB_CONFIG, cfg.wb_device, cfg.wb_bus))
    return shorts


def wb_shorts_of(conf, device, bus) -> set:
    shorts = set()
    for gateway in conf.get("gateways", []):
        if gateway.get("device_id") != device:
            continue
        buses = gateway.get("buses", [])
        if len(buses) < bus:
            continue
        for dev in buses[bus - 1].get("devices", []):
            if not dev.get("dali2") and isinstance(dev.get("short"), int):
                shorts.add(dev["short"])
    return shorts


def used_groups(devices, matrix_rows):
    used = set()
    for dev in devices:
        mask = dev.get("groups_membership")
        if mask is None:
            return None
        used |= {g for g in range(GROUP_COUNT) if mask >> g & 1}
    for row in matrix_rows:
        for key in ("desired", "applied"):
            used |= {g for g, bit in enumerate(row.get(key) or []) if bit}
    return used


def free_groups(used) -> list:
    return [] if used is None else [g for g in range(GROUP_COUNT) if g not in used]


def group_address(group) -> int:
    return GROUP_ADDRESS_BASE | (group << 1) | SELECTOR_BIT


def answered_yes(resp) -> bool:
    return bool(resp.get("backward_violation")) or \
        (bool(resp.get("success")) and resp.get("backward_frame") == YES)


def answered_cleanly(resp) -> bool:
    return bool(resp.get("success")) and not resp.get("backward_violation")


def answered_any(resp) -> bool:
    return bool(resp.get("success")) or bool(resp.get("backward_violation"))


def _counters(source, names, where) -> tuple:
    missing = [name for name in names if name not in source]
    if missing:
        raise VirtualGearError("%s lacks %s, so the group proof cannot see the wire"
                               % (where, ", ".join(missing)))
    return tuple(source[name] for name in names)


def wire_window(api) -> tuple:
    dali = api.stats().get("dali") or {}
    wire = api.diagnostics().get("dali_wire") or {}
    contended = (_counters(dali, CONTENTION_COUNTERS, "stats.dali")
                 + _counters(wire, WIRE_CONTENTION, "diagnostics.dali_wire"))
    damaged = (_counters(dali, ANSWER_DAMAGE, "stats.dali")
               + _counters(wire, WIRE_DAMAGE, "diagnostics.dali_wire"))
    return contended, damaged, sum(_counters(wire, (FRAMES_SENT,), "diagnostics.dali_wire")[0])


def _replies(api, group) -> list:
    return [api.cmd_wire(group_address(group), QUERY_CONTROL_GEAR_PRESENT)
            for _ in range(PROOF_PROBES)]


def _windowed_replies(api, group):
    before = wire_window(api)
    replies = _replies(api, group)
    after = wire_window(api)
    contended = after[0] != before[0] or after[2] - before[2] != PROOF_PROBES
    return replies, contended, after[1] != before[1]


def await_quiet_wire(api):
    settled(lambda: wire_window(api)[0], QUIET_WIRE_S, QUIET_WAIT_MAX_S, QUIET_POLL_S,
            key=tuple)


def _is_control(api, group) -> bool:
    contended = False
    for _ in range(PROOF_ATTEMPTS):
        if contended:
            await_quiet_wire(api)
        replies, contended, damaged = _windowed_replies(api, group)
        if not (contended or damaged):
            return all(answered_yes(r) for r in replies)
    return False


def _is_occupied(api, group) -> bool:
    for attempt in range(PROOF_ATTEMPTS):
        if attempt:
            await_quiet_wire(api)
        replies, contended, damaged = _windowed_replies(api, group)
        if any(answered_any(r) for r in replies):
            return True
        if not contended:
            return damaged
    raise VirtualGearError("another transmitter shared the wire, or a probe did not go out, "
                           "during every proof of group %d, each retried after waiting up to "
                           "%ds for a quiet wire (%s moved), so silence proved nothing"
                           % (group, QUIET_WAIT_MAX_S,
                              ", ".join(CONTENTION_COUNTERS + WIRE_CONTENTION)))


def prove_groups_empty(api, groups, used):
    if not groups:
        return None
    control = next((g for g in sorted(used) if _is_control(api, g)), None)
    if control is None:
        raise VirtualGearError(
            "no group live lamps hold answered QUERY CONTROL GEAR PRESENT on every probe of a "
            "quiet window, so silence from the free groups would prove nothing")
    occupied = [g for g in groups if _is_occupied(api, g)]
    if occupied:
        raise VirtualGearError("groups %s answer on the wire, or garble an answer: real gear may "
                               "sit in them" % occupied)
    return control


@dataclass(frozen=True)
class SessionScope:
    groups: frozenset
    lamps: frozenset
    park: frozenset
    names: frozenset

    @classmethod
    def of(cls, groups, lamps, park):
        return cls(frozenset(groups), frozenset(lamps), frozenset(park),
                   frozenset(PARK_VL_NAME % s for s in park))


def rule_references(compiled, adapter) -> dict:
    refs = {"group": set(), "lamp": set(), "device": set()}
    _collect(compiled, adapter, refs)
    return refs


def _collect(node, adapter, refs):
    if isinstance(node, list):
        for item in node:
            _collect(item, adapter, refs)
    elif isinstance(node, dict):
        _note(node, adapter, refs)
        for value in node.values():
            _collect(value, adapter, refs)


def _ours(ref, adapter):
    return isinstance(ref, dict) and ref.get("adapter_id", adapter) == adapter


def _note(node, adapter, refs):
    scope = {LAMP_SCOPE: "lamp", GROUP_SCOPE: "group"}.get(node.get("scope"))
    if scope and isinstance(node.get("id"), int) and _ours(node, adapter):
        refs[scope].add(node["id"])
    for key in ("lamp", "group"):
        ref = node.get(key)
        if _ours(ref, adapter) and isinstance(ref.get("id"), int):
            refs[key].add(ref["id"])
    device = node.get("device")
    if _ours(device, adapter) and isinstance(device.get("short_address"), int):
        refs["device"].add(device["short_address"])


def text_references(source, group_ids) -> dict:
    text = source or ""
    return {"group": {int(g) for g in RULE_GROUP.findall(text)}
            | {group_ids[n] for n in RULE_GROUP_NAME.findall(text) if n in group_ids},
            "lamp": {int(n) for n in RULE_LAMP_ID.findall(text)},
            "lamp_name": set(RULE_LAMP.findall(text)),
            "device": {int(d) for d in RULE_DEVICE.findall(text)}}


def rules_conflicts(source, compiled, scope, group_ids, adapter) -> list:
    refs = text_references(source, group_ids)
    for kind, found in rule_references(compiled, adapter).items():
        refs[kind] |= found
    out = ["the owner's rules use group %d" % g for g in sorted(refs["group"] & scope.groups)]
    out += ["the owner's rules use lamp %d" % n for n in sorted(refs["lamp"] & scope.lamps)]
    out += ["the owner's rules name lamp %r" % n
            for n in sorted(refs["lamp_name"] & scope.names)]
    out += ["the owner's rules watch device %d" % d
            for d in sorted(refs["device"] & scope.park)]
    return out


def owner_rule_conflicts(api, scope) -> list:
    group_ids = {g.get("name"): g["group_id"] for g in api.groups.list()["groups"]}
    compiled = api._req("GET", "rules?format=json").get("rules")
    return rules_conflicts(api.rules_get().get("source"), compiled, scope, group_ids,
                           api.adapter)


def owner_rule_groups(api) -> set:
    group_ids = {g.get("name"): g["group_id"] for g in api.groups.list()["groups"]}
    compiled = api._req("GET", "rules?format=json").get("rules")
    return (text_references(api.rules_get().get("source"), group_ids)["group"]
            | rule_references(compiled, api.adapter)["group"])


def real_tier_conflicts(api, allowed) -> list:
    lamps = [v for v in api.vlamps.list_unfiltered()["virtual_lamps"]
             if (v.get("binding") or {}).get("physical_short_address") in allowed]
    ids = {v["virtual_lamp_id"] for v in lamps}
    groups, problems = set(), []
    for dev in api.devices_unfiltered()["physical_devices"]:
        if dev["short_address"] not in allowed:
            continue
        mask = dev.get("groups_membership")
        if mask is None:
            problems.append("SA%d reports no group membership, so an owner rule on its "
                            "groups cannot be ruled out" % dev["short_address"])
            continue
        groups |= {g for g in range(GROUP_COUNT) if mask >> g & 1}
    for row in api.groups.matrix().get("rows", []):
        if row["virtual_lamp_id"] in ids:
            for key in ("desired", "applied"):
                groups |= {g for g, bit in enumerate(row.get(key) or []) if bit}
    scope = SessionScope(frozenset(groups), frozenset(ids), frozenset(allowed),
                         frozenset(v["name"] for v in lamps if v.get("name")))
    return problems + owner_rule_conflicts(api, scope)


def free_vl_ids(existing, count) -> list:
    free = [i for i in range(VL_ID_LIMIT) if i not in set(existing)]
    if len(free) < count:
        raise VirtualGearError("%d virtual lamps needed, %d ids are free" % (count, len(free)))
    return free[-count:]


def retained(cfg, prefixes) -> dict:
    topics = " ".join("-t '%s/#'" % p for p in prefixes)
    out = subprocess.run(
        ["ssh", *remote_serial.SSH_OPTS, cfg.wb_ssh,
         "mosquitto_sub -h localhost %s -v --retained-only -W %d 2>/dev/null; true"
         % (topics, RETAINED_WAIT_S)],
        capture_output=True, text=True, timeout=SSH_TIMEOUT_S)
    found = {}
    for line in out.stdout.splitlines():
        topic, _, payload = line.partition(" ")
        found[topic] = payload
    return found


def retained_residue(before, now, discovery_prefix) -> list:
    out = ["retained MQTT topic appeared: %s" % t for t in sorted(set(now) - set(before))]
    out += ["retained MQTT topic vanished: %s" % t for t in sorted(set(before) - set(now))]
    out += ["retained MQTT config changed: %s" % t for t in sorted(set(before) & set(now))
            if discovery_prefix and t.startswith(discovery_prefix + "/")
            and before[t] != now[t]]
    return out


class VirtualSession:
    def __init__(self, cfg, api, sim, owner_shorts=(), park=DEFAULT_PARK, log=print):
        self.cfg, self.api, self.sim = cfg, api, sim
        self.owner_shorts, self.shape, self.log = frozenset(owner_shorts), park, log
        self.ledger = Ledger.of(cfg)

    def plan(self):
        registry, wb = registry_shorts(self.api), wb_shorts(self.cfg)
        reserved = reserve(registry, wb, self.owner_shorts)
        return reserved, park_shorts(reserved, sum(self.shape)), wb

    def precheck(self, groups, park):
        problems = []
        if remote_serial.enabled(self.cfg) and not serialmon.alive(self.cfg):
            problems.append("the DUT's serial monitor is down: no tripwire without it")
        scope = SessionScope.of(groups, free_vl_ids(vl_ids(self.api), len(park)), park)
        return problems + owner_rule_conflicts(self.api, scope)

    def open(self):
        if self.ledger.exists():
            raise VirtualGearError("a previous virtual-gear session left %s: `hil state "
                                   "restore` finishes its teardown" % self.ledger.path)
        reserved, park, wb = self.plan()
        used = used_groups(self.api.devices_unfiltered()["physical_devices"],
                           self.api.groups.matrix().get("rows", []))
        groups = free_groups(used)
        problems = self.precheck(groups, park)
        if problems:
            raise VirtualGearError("; ".join(problems))
        control = prove_groups_empty(self.api, groups, used)
        ha = self._ha_prefixes()
        self.ledger.update(opened_at=time.strftime("%Y-%m-%dT%H:%M:%S"), phase="building",
                           reserve=sorted(reserved), park=park, shape=list(self.shape),
                           groups=groups,
                           positive_control=control, wb_before=sorted(wb),
                           vl_before=vl_ids(self.api), ha_prefixes=ha,
                           ha_before=self._retained(ha))
        self._build(reserved, park)
        self._neutralise(groups)
        self._enrol(park, groups)
        self.ledger.update(phase="open")
        return self.ledger.data

    def _ha_prefixes(self):
        ha = self.api.ha.get()
        if not ha.get("enabled"):
            return {}
        return {key: ha.get(key) for key in ("discovery_prefix", "state_topic_prefix")
                if ha.get(key)}

    def _retained(self, prefixes):
        if not prefixes:
            return {}
        found = retained(self.cfg, sorted(prefixes.values()))
        if not found:
            raise VirtualGearError("Home Assistant is enabled, yet no retained topic under %s "
                                   "was read: the HA check would be blind"
                                   % sorted(prefixes.values()))
        return found

    def _build(self, reserved, park):
        self.sim.reserve(reserved)
        self.sim.fleet(park[0], *self.shape)
        shown = sorted(row["short"] for row in self.sim.show())
        if shown != park:
            raise VirtualGearError("the emulator laid its fleet out at %s, the plan was %s"
                                   % (spell(shown), spell(park)))
        self._ladder(park)

    def _ladder(self, park):
        first = park[0]
        before = self.sim.stats()
        self.sim.enable(first)
        reply = self.api.cmd(first, QUERY_STATUS)
        if not answered_cleanly(reply):
            raise VirtualGearError("SA%d does not answer the controller's QUERY STATUS "
                                   "cleanly: %r" % (first, reply))
        after = self.sim.stats()
        moved = {key: after.get(key, 0) - before.get(key, 0) for key in LADDER_COUNTERS}
        if moved["sent"] < 1 or moved["late"] or moved["expired"]:
            raise VirtualGearError("the first answer left the window: %r" % moved)
        for short in park[1:LADDER_FIRST]:
            self.sim.enable(short)
        self.sim.enable("all")

    def _neutralise(self, groups):
        flags = {}
        for group in groups:
            flags[str(group)] = self.api.groups.get(group).get("ha_entity_enabled")
            self.ledger.update(group_flags=flags)
            self.api.groups.patch(group, {"ha_entity_enabled": False})

    def _enrol(self, park, groups):
        self.api.wait_op(self.api.discovery(SCAN_MODE))
        missing = sorted(set(park) - registry_shorts(self.api))
        if missing:
            raise VirtualGearError("the scan did not enrol %s" % spell(missing))
        stray = [row["short"] for row in self.sim.show()
                 if row["short"] in park and row["groups"] & ~group_mask(groups)]
        if stray:
            raise VirtualGearError("emulated gear %s hold groups outside %s" % (stray, groups))
        self._prove_emulated(park)
        ids = free_vl_ids(self.ledger.data.get("vl_before", []), len(park))
        for lamp_id, short in zip(ids, park):
            self.ledger.append("created_vls", lamp_id)
            self.api.vlamps.patch(lamp_id, {"name": PARK_VL_NAME % short, "ha_entity_enabled": False})
            self.api.vlamps.bind(lamp_id, short)
        self.ledger.update(vl_of_short={str(s): i for i, s in zip(ids, park)})

    def _prove_emulated(self, park):
        for short in park:
            self.api.attr_read_checked(short, groups=IDENTITY_GROUPS, banks=IDENTITY_BANKS)
        gtins = {d["short_address"]: d.get("gtin")
                 for d in self.api.devices_unfiltered()["physical_devices"]}
        foreign = [short for short in park if not emulated_gtin(gtins.get(short))]
        if foreign:
            raise VirtualGearError("SA%s carry no gear-emulator GTIN after their identity read: "
                                   "real gear may answer at the park" % spell(foreign))

    def close(self):
        if not self.ledger.exists():
            return []
        residue = []
        for label, step in self._teardown_steps():
            try:
                step()
            except Exception as exc:
                residue.append("%s failed: %s" % (label, exc))
                self.log("virtual gear: %s failed: %s" % (label, exc))
        try:
            residue += self.residual()
        except Exception as exc:
            residue.append("the residue check failed: %s" % exc)
        if not residue:
            self.ledger.remove()
        return residue

    def _teardown_steps(self):
        data = self.ledger.data
        steps = [("silencing the emulated fleet", self._silence)]
        steps += [("deleting VL%d" % i, partial(_ignore_missing, partial(self._delete_lamp, i)))
                  for i in data.get("created_vls", [])]
        steps += [("forgetting SA%d" % s, partial(_ignore_missing, partial(self._forget, s)))
                  for s in data.get("park", [])]
        steps += [("restoring group %s's HA flag" % g, partial(self._restore_flag, int(g), flag))
                  for g, flag in (data.get("group_flags") or {}).items()]
        return steps

    def _delete_lamp(self, lamp_id):
        lamp = self.api.vlamps.get(lamp_id)
        bound = (lamp.get("binding") or {}).get("physical_short_address")
        if not park_lamp(lamp.get("name"), bound, self.ledger.data.get("park", [])):
            raise VirtualGearError("VL%d is now %r bound to %r, not the lamp this session made: "
                                   "left as it is" % (lamp_id, lamp.get("name"), bound))
        with self.api.guard.deleting_created(self.ledger.data.get("created_vls", [])):
            self.api.vlamps.delete(lamp_id)

    def _forget(self, short):
        if short not in registry_shorts(self.api):
            return
        data = self.ledger.data
        with self.api.guard.forgetting_emulated(data.get("park", []), data.get("reserve", [])):
            self.api.device_forget(short)

    def _silence(self):
        if self.sim is None:
            self.log("virtual gear: no emulator console, so no fleet to silence")
            return
        self.sim.disable("all")

    def _restore_flag(self, group, flag):
        if not isinstance(flag, bool):
            raise VirtualGearError("group %d's HA flag was %r before the session; it stays "
                                   "hidden" % (group, flag))
        self.api.groups.patch(group, {"ha_entity_enabled": flag})

    def residual(self):
        data, out = self.ledger.data, []
        left = registry_shorts(self.api) & set(data.get("park", []))
        if left:
            out.append("emulated gear still registered: %s" % spell(left))
        vls = set(vl_ids(self.api))
        out += ["VL%d created by the session still exists" % i
                for i in data.get("created_vls", []) if i in vls]
        wb = wb_shorts(self.cfg)
        if sorted(wb) != data.get("wb_before"):
            out.append("the WB master's device list changed: %s -> %s"
                       % (spell(data.get("wb_before", [])), spell(wb)))
        prefixes = data.get("ha_prefixes") or {}
        return out + retained_residue(data.get("ha_before") or {}, self._retained(prefixes),
                                      prefixes.get("discovery_prefix"))


def teardown(cfg, api, log=print) -> list:
    if not Ledger.of(cfg).exists():
        return []
    try:
        sim = GearSim(cfg.peer())
    except (GearSimUnavailable, RuntimeError) as exc:
        log("virtual gear: no emulator console (%s)" % exc)
        sim = None
    return VirtualSession(cfg, api, sim, log=log).close()


def park_lamp(name, bound, park) -> bool:
    named = [short for short in park if name == PARK_VL_NAME % short]
    return bool(named) and bound in (None, named[0])


def group_mask(groups) -> int:
    mask = 0
    for group in groups:
        mask |= 1 << group
    return mask


def _ignore_missing(action):
    try:
        action()
    except Exception as exc:
        if getattr(exc, "status", None) != 404:
            raise
