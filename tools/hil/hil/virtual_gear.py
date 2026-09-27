import json
import os
import re
import subprocess
import time
from functools import partial
from pathlib import Path

from hil import remote_serial, role, serialmon
from hil.gearsim import GearSim, GearSimUnavailable
from hil.lamp_guard import spell

LEDGER = role.LEDGER

SHORT_COUNT = 64
GROUP_COUNT = 16
VL_ID_LIMIT = 64
RESERVE_FLOOR = 16
DEFAULT_PARK = (4, 4, 4)
LADDER_FIRST = 4
LADDER_COUNTERS = ("sent", "late", "expired")

GROUP_ADDRESS_BASE = 0x80
SELECTOR_BIT = 0x01
QUERY_STATUS = 0x90
QUERY_CONTROL_GEAR_PRESENT = 0x91
YES = 0xFF
PROOF_PROBES = 3
PROOF_ATTEMPTS = 3
CONTENTION_COUNTERS = ("collision_restarts_total", "foreign_frames_total")

SCAN_MODE = "scan_known_short_addresses"
PARK_VL_NAME = "virtual gear SA%d"
WB_CONFIG = "/etc/wb-mqtt-dali.conf"
SSH_TIMEOUT_S = 30
RETAINED_WAIT_S = 3

RULE_GROUP = re.compile(r"\bgroup\((\d+)\)")
RULE_LAMP = re.compile(r"\blamp\(\"([^\"]*)\"\)")


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
        tmp = self.path.with_name(self.path.name + ".tmp")
        tmp.write_text(json.dumps(self.data, indent=1, sort_keys=True, ensure_ascii=False))
        os.replace(tmp, self.path)

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


def contention(api) -> tuple:
    dali = api.stats().get("dali") or {}
    return tuple(dali.get(name, 0) for name in CONTENTION_COUNTERS)


def _answers(api, group) -> list:
    return [answered_yes(api.cmd_wire(group_address(group), QUERY_CONTROL_GEAR_PRESENT))
            for _ in range(PROOF_PROBES)]


def _probe_groups(api, groups, used):
    control = next((g for g in sorted(used) if all(_answers(api, g))), None)
    occupied = [g for g in groups if any(_answers(api, g))]
    return control, occupied


def prove_groups_empty(api, groups, used):
    if not groups:
        return None
    for _ in range(PROOF_ATTEMPTS):
        before = contention(api)
        control, occupied = _probe_groups(api, groups, used)
        if contention(api) != before:
            continue
        if control is None:
            raise VirtualGearError(
                "no group live lamps hold answered QUERY CONTROL GEAR PRESENT on every probe, "
                "so silence from the free groups would prove nothing")
        if occupied:
            raise VirtualGearError("groups %s answer on the wire: real gear sits in them"
                                   % occupied)
        return control
    raise VirtualGearError("another transmitter shared the wire during every group proof "
                           "(%s moved), so silence proved nothing" % ", ".join(CONTENTION_COUNTERS))


def rules_conflicts(source, groups, names) -> list:
    out = ["the owner's rules use group %d" % int(g)
           for g in RULE_GROUP.findall(source or "") if int(g) in set(groups)]
    out += ["the owner's rules name lamp %r" % n
            for n in RULE_LAMP.findall(source or "") if n in set(names)]
    return out


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

    def precheck(self, groups, park_names=()):
        problems = []
        if remote_serial.enabled(self.cfg) and not serialmon.alive(self.cfg):
            problems.append("the DUT's serial monitor is down: no tripwire without it")
        problems += rules_conflicts(self.api.rules_get().get("source"), groups, park_names)
        return problems

    def open(self):
        if self.ledger.exists():
            raise VirtualGearError("a previous virtual-gear session left %s: `hil state "
                                   "restore` finishes its teardown" % self.ledger.path)
        reserved, park, wb = self.plan()
        used = used_groups(self.api.devices_unfiltered()["physical_devices"],
                           self.api.groups.matrix().get("rows", []))
        groups = free_groups(used)
        problems = self.precheck(groups, [PARK_VL_NAME % s for s in park])
        if problems:
            raise VirtualGearError("; ".join(problems))
        control = prove_groups_empty(self.api, groups, used)
        ha = self._ha_prefixes()
        self.ledger.update(opened_at=time.strftime("%Y-%m-%dT%H:%M:%S"), phase="building",
                           reserve=sorted(reserved), park=park, groups=groups,
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
        policy = self.api._req("GET", "policies")
        if policy.get("apply_on_discovery") and policy.get("manages_anything"):
            self.ledger.update(policy_rearm=True)
            self.api._req("PATCH", "policies", {"apply_on_discovery": False})
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
        ids = free_vl_ids(self.ledger.data.get("vl_before", []), len(park))
        for lamp_id, short in zip(ids, park):
            self.ledger.append("created_vls", lamp_id)
            self.api.vlamps.patch(lamp_id, {"name": PARK_VL_NAME % short, "ha_entity_enabled": False})
            self.api.vlamps.bind(lamp_id, short)
        self.ledger.update(vl_of_short={str(s): i for i, s in zip(ids, park)})

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
        steps += [("deleting VL%d" % i, partial(_ignore_missing, partial(self.api.vlamps.delete, i)))
                  for i in data.get("created_vls", [])]
        steps += [("forgetting SA%d" % s, partial(_ignore_missing, partial(self.api.device_forget, s)))
                  for s in data.get("park", [])]
        steps += [("restoring group %s's HA flag" % g, partial(self._restore_flag, int(g), flag))
                  for g, flag in (data.get("group_flags") or {}).items()]
        if data.get("policy_rearm"):
            steps.append(("re-arming apply-on-discovery", partial(
                self.api._req, "PATCH", "policies", {"apply_on_discovery": True})))
        return steps

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
