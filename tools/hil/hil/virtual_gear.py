import json
import os
import re
import subprocess
import time
from pathlib import Path

from hil import remote_serial, role, serialmon

LEDGER = role.LEDGER

SHORT_COUNT = 64
GROUP_COUNT = 16
VL_ID_LIMIT = 64
RESERVE_FLOOR = 16
DEFAULT_PARK = (4, 4, 4)
LADDER_FIRST = 4

GROUP_ADDRESS_BASE = 0x80
SELECTOR_BIT = 0x01
QUERY_STATUS = 0x90
QUERY_CONTROL_GEAR_PRESENT = 0x91
YES = 0xFF

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


def spell(shorts) -> str:
    ordered, runs = sorted(set(shorts)), []
    for short in ordered:
        if runs and short == runs[-1][1] + 1:
            runs[-1][1] = short
        else:
            runs.append([short, short])
    return ",".join(str(a) if a == b else "%d-%d" % (a, b) for a, b in runs)


def reserve(registry, wb, owner) -> frozenset:
    return frozenset(range(RESERVE_FLOOR)) | frozenset(registry) | frozenset(wb) | frozenset(owner)


def park_shorts(reserved, count) -> list:
    free = [s for s in range(SHORT_COUNT) if s not in reserved]
    if len(free) < count:
        raise VirtualGearError("%d gear asked, %d addresses are free of the reserve"
                               % (count, len(free)))
    base = free[0]
    return [s for s in free if s >= base][:count]


def registry_shorts(api) -> set:
    return {d["short_address"] for d in api.devices_unfiltered()["physical_devices"]}


def wb_shorts(cfg) -> set:
    out = subprocess.run(["ssh", *remote_serial.SSH_OPTS, cfg.wb_ssh, "cat %s" % WB_CONFIG],
                         capture_output=True, text=True, timeout=SSH_TIMEOUT_S)
    if out.returncode != 0:
        raise VirtualGearError("could not read %s on %s: %s"
                               % (WB_CONFIG, cfg.wb_ssh, out.stderr.strip()))
    return wb_shorts_of(json.loads(out.stdout), cfg.wb_device, cfg.wb_bus)


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
        for group, desired in enumerate(row.get("desired") or []):
            if desired:
                used.add(group)
    return used


def free_groups(used) -> list:
    return [] if used is None else [g for g in range(GROUP_COUNT) if g not in used]


def group_address(group) -> int:
    return GROUP_ADDRESS_BASE | (group << 1) | SELECTOR_BIT


def answered_yes(resp) -> bool:
    return bool(resp.get("backward_violation")) or \
        (bool(resp.get("success")) and resp.get("backward_frame") == YES)


def prove_groups_empty(api, groups, used):
    controls = [g for g in sorted(used or ()) if answered_yes(
        api.cmd_wire(group_address(g), QUERY_CONTROL_GEAR_PRESENT))]
    if not controls:
        raise VirtualGearError(
            "no group live lamps hold answered QUERY CONTROL GEAR PRESENT, so silence "
            "from the free groups would prove nothing")
    occupied = [g for g in groups if answered_yes(
        api.cmd_wire(group_address(g), QUERY_CONTROL_GEAR_PRESENT))]
    if occupied:
        raise VirtualGearError("groups %s answer on the wire: real gear sits in them"
                               % occupied)
    return controls[0]


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


class VirtualSession:
    def __init__(self, cfg, api, peer_cfg, sim, owner_shorts=(), park=DEFAULT_PARK, log=print):
        self.cfg, self.api, self.peer_cfg, self.sim = cfg, api, peer_cfg, sim
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
        self.ledger.update(opened_at=time.strftime("%Y-%m-%dT%H:%M:%S"), phase="building",
                           reserve=sorted(reserved), park=park, groups=groups,
                           positive_control=control, wb_before=sorted(wb),
                           vl_before=[v["virtual_lamp_id"] for v in
                                      self.api.vlamps.list()["virtual_lamps"]],
                           ha_before=self._retained())
        self._build(reserved, park)
        self._neutralise(groups)
        self._enrol(park, groups)
        self.ledger.update(phase="open")
        return self.ledger.data

    def _retained(self):
        ha = self.api.ha.get()
        if not ha.get("enabled"):
            return {}
        prefixes = [p for p in (ha.get("discovery_prefix"), ha.get("state_topic_prefix")) if p]
        found = retained(self.cfg, prefixes)
        if not found:
            raise VirtualGearError("Home Assistant is enabled, yet no retained topic under %s "
                                   "was read: the HA check would be blind" % prefixes)
        return found

    def _build(self, reserved, park):
        self.sim.reserve(spell(reserved))
        self.sim.fleet(park[0], *self.shape)
        shown = sorted(row["short"] for row in self.sim.show())
        if shown != park:
            raise VirtualGearError("the emulator laid its fleet out at %s, the plan was %s"
                                   % (spell(shown), spell(park)))
        self._ladder(park)

    def _ladder(self, park):
        first = park[0]
        self.sim.enable(first)
        if not answered_status(self.api.cmd(first, QUERY_STATUS)):
            raise VirtualGearError("SA%d does not answer the controller's QUERY STATUS" % first)
        stats = self.sim.stats()
        if stats.get("sent", 0) < 1 or stats.get("late", 0) or stats.get("expired", 0):
            raise VirtualGearError("the first answer left the window: %r" % stats)
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
        present = {d["short_address"]: d for d in self.api.devices_unfiltered()["physical_devices"]}
        missing = [s for s in park if s not in present]
        if missing:
            raise VirtualGearError("the scan did not enrol %s" % spell(missing))
        stray = [s for s in park if (present[s].get("groups_membership") or 0) & ~group_mask(groups)]
        if stray:
            raise VirtualGearError("emulated gear %s hold groups outside %s" % (stray, groups))
        ids = free_vl_ids(self.ledger.data.get("vl_before", []), len(park))
        for lamp_id, short in zip(ids, park):
            self.api.vlamps.patch(lamp_id, {"name": PARK_VL_NAME % short, "ha_entity_enabled": False})
            self.ledger.append("created_vls", lamp_id)
            self.api.vlamps.bind(lamp_id, short)
        self.ledger.update(vl_of_short={str(s): i for i, s in zip(ids, park)})

    def close(self):
        if not self.ledger.exists():
            return []
        data = self.ledger.data
        try:
            self.sim.disable("all")
        except Exception as exc:
            self.log("virtual gear: the emulator did not go quiet (%s)" % exc)
        for lamp_id in data.get("created_vls", []):
            _ignore_missing(lambda: self.api.vlamps.delete(lamp_id))
        for short in data.get("park", []):
            _ignore_missing(lambda: self.api.device_forget(short))
        for group, flag in (data.get("group_flags") or {}).items():
            self.api.groups.patch(int(group), {"ha_entity_enabled": flag})
        if data.get("policy_rearm"):
            self.api._req("PATCH", "policies", {"apply_on_discovery": True})
        residual = self.residual()
        if not residual:
            self.ledger.remove()
        return residual

    def residual(self):
        data, out = self.ledger.data, []
        left = registry_shorts(self.api) & set(data.get("park", []))
        if left:
            out.append("emulated gear still registered: %s" % spell(left))
        vls = {v["virtual_lamp_id"] for v in self.api.vlamps.list()["virtual_lamps"]}
        out += ["VL%d created by the session still exists" % i
                for i in data.get("created_vls", []) if i in vls]
        wb = wb_shorts(self.cfg)
        if sorted(wb) != data.get("wb_before"):
            out.append("the WB master's device list changed: %s -> %s"
                       % (spell(data.get("wb_before", [])), spell(wb)))
        now = self._retained()
        changed = sorted(set(now.items()) ^ set((data.get("ha_before") or {}).items()))
        out += ["retained MQTT topic differs: %s" % topic for topic in
                sorted({topic for topic, _ in changed})]
        return out


def group_mask(groups) -> int:
    mask = 0
    for group in groups:
        mask |= 1 << group
    return mask


def answered_status(resp) -> bool:
    return bool(resp.get("success")) or bool(resp.get("backward_violation"))


def _ignore_missing(action):
    try:
        action()
    except Exception as exc:
        if getattr(exc, "status", None) != 404:
            raise
