import json
import subprocess
import sys
import time
from pathlib import Path

import requests

from hil import flash, remote_serial, serialmon

ROLE_PIN = "role.pin"
ROLE_CONTROLLER = "controller"
ROLE_GEAR_SIM = "gear-sim"
CONTROLLER_ROLES = ("active", "standby")
ROLE_RETURN_TIMEOUT_S = 120
ROLE_POLL_S = 2.0
REQUEST_TIMEOUT_S = 3
SSH_TIMEOUT_S = 20
REPLICATION_WAIT_S = 120
LEDGER = "virtual_gear.json"
FLASHER = "esptool"
STATE_PENDING_VERIFY = "pending_verify"


class RoleError(RuntimeError):
    pass


def pin_path(peer_cfg) -> Path:
    return Path(peer_cfg.state_dir) / ROLE_PIN


def current(peer_cfg) -> dict:
    try:
        return json.loads(pin_path(peer_cfg).read_text())
    except (OSError, ValueError):
        return {"role": ROLE_CONTROLLER}


def is_gear_sim(peer_cfg) -> bool:
    return current(peer_cfg).get("role") == ROLE_GEAR_SIM


def _record(peer_cfg, role, **fields):
    path = pin_path(peer_cfg)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(dict(fields, role=role, at=time.strftime(
        "%Y-%m-%dT%H:%M:%S", time.gmtime())), sort_keys=True) + "\n")


def _get(base, path):
    try:
        r = requests.get(base.rstrip("/") + path, timeout=REQUEST_TIMEOUT_S)
        return r.json() if r.ok else None
    except (requests.RequestException, ValueError):
        return None


def controller_health(cfg):
    health = _get(cfg.base, "/api/v1/health") if cfg.base else None
    return health if health and health.get("role") in CONTROLLER_ROLES else None


def foreign_flashers(cfg) -> list:
    out = []
    local = subprocess.run(["pgrep", "-fl", FLASHER], capture_output=True, text=True)
    out += ["here: %s" % line for line in local.stdout.splitlines() if line.strip()]
    remote = subprocess.run(["ssh", *remote_serial.SSH_OPTS, cfg.wb_ssh,
                             "pgrep -fa %s; true" % FLASHER],
                            capture_output=True, text=True, timeout=SSH_TIMEOUT_S)
    out += ["on %s: %s" % (cfg.wb_ssh, line) for line in remote.stdout.splitlines()
            if line.strip() and "pgrep" not in line]
    return out


def preconditions(dut_cfg, peer_cfg) -> list:
    problems = []
    dut, peer = controller_health(dut_cfg), controller_health(peer_cfg)
    if not dut or dut.get("role") != "active":
        problems.append("the DUT (%s) is not active: %r" % (dut_cfg.base, dut))
    if not peer or peer.get("role") != "standby":
        problems.append("the peer (%s) is not a standby controller: %r" % (peer_cfg.base, peer))
    if (Path(dut_cfg.state_dir) / LEDGER).exists():
        problems.append("a virtual-gear session left %s: finish its teardown first"
                        % (Path(dut_cfg.state_dir) / LEDGER))
    if peer_cfg.serial_remote and peer_cfg.serial_remote == dut_cfg.serial_remote:
        problems.append("the peer's serial bridge is the DUT's (%s)" % peer_cfg.serial_remote)
    if remote_serial.enabled(peer_cfg) and not serialmon.alive(peer_cfg):
        problems.append("the peer's serial monitor is not running: start it with "
                        "`hil --peer monitor start` while the peer is still a controller "
                        "(starting a bridge that is down resets the board)")
    problems += ["another flasher runs %s" % f for f in foreign_flashers(peer_cfg)]
    return problems


def to_gear_sim(dut_cfg, peer_cfg, via=flash.VIA_OTA, log=print) -> int:
    if is_gear_sim(peer_cfg) and controller_health(peer_cfg) is None:
        log("the peer already runs the gear emulator (%s)" % current(peer_cfg))
        return 0
    problems = preconditions(dut_cfg, peer_cfg)
    if problems:
        for problem in problems:
            print("role: %s" % problem, file=sys.stderr)
        return 2
    rc = flash.run(peer_cfg, image=flash.GEAR_SIM, via=via, on_delivered=lambda: _record(
        peer_cfg, ROLE_GEAR_SIM, via=via, confirmed=False))
    if rc != 0:
        return rc
    _record(peer_cfg, ROLE_GEAR_SIM, via=via, confirmed=True)
    log("the peer runs the gear emulator (%s); `hil --peer role controller` hands it back"
        % via)
    return 0


def to_controller(dut_cfg, peer_cfg, via=None, log=print) -> int:
    if (Path(dut_cfg.state_dir) / LEDGER).exists():
        print("role: a virtual-gear session is still open (%s): its teardown must "
              "clean the DUT's registry before the standby pulls it"
              % (Path(dut_cfg.state_dir) / LEDGER), file=sys.stderr)
        return 2
    if controller_health(peer_cfg) is None:
        try:
            rc = _hand_back(peer_cfg, via or return_via(peer_cfg))
        except (OSError, remote_serial.RemoteError) as exc:
            print("role: the peer's bridge failed: %s" % exc, file=sys.stderr)
            return 1
        if rc != 0:
            return rc
    _record(peer_cfg, ROLE_CONTROLLER)
    log("the peer runs the controller again; comparing its registry with the DUT's")
    return compare_registries(dut_cfg, peer_cfg, log)


def return_via(peer_cfg) -> str:
    banner = flash.ready_line(peer_cfg, 0)
    if banner is not None:
        return flash.VIA_OTA if banner[2] == STATE_PENDING_VERIFY else flash.VIA_WB
    return current(peer_cfg).get("via", flash.VIA_WB)


def _hand_back(peer_cfg, via) -> int:
    if via == flash.VIA_WB:
        return flash.run(peer_cfg, image=flash.CONTROLLER, via=flash.VIA_WB)
    remote_serial.control(peer_cfg, "run")
    if _await_controller(peer_cfg):
        return 0
    print("role: a reset did not bring the controller back, so the bootloader kept the "
          "emulator: `hil --peer role controller --via wb` writes the controller",
          file=sys.stderr)
    return 1


def _await_controller(peer_cfg, timeout_s=ROLE_RETURN_TIMEOUT_S) -> bool:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        health = controller_health(peer_cfg)
        if health and health.get("role") == "standby":
            return True
        time.sleep(ROLE_POLL_S)
    print("role: the peer did not answer as a standby controller within %ds"
          % timeout_s, file=sys.stderr)
    return False


def _slices(cfg):
    rows = _get(cfg.base, "/api/v1/config/slices")
    rows = rows.get("slices", rows) if isinstance(rows, dict) else rows
    return sorted((r.get("name"), r.get("crc32")) for r in rows or [])


def _converged(dut_cfg, peer_cfg, timeout_s=REPLICATION_WAIT_S) -> bool:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        dut = _slices(dut_cfg)
        if dut and dut == _slices(peer_cfg):
            return True
        time.sleep(ROLE_POLL_S)
    return False


def _registry(cfg):
    adapter = "/api/v1/adapters/%d" % cfg.adapter
    devices = _get(cfg.base, adapter + "/physical-devices") or {}
    lamps = _get(cfg.base, adapter + "/virtual-lamps") or {}
    shorts = sorted(d["short_address"] for d in devices.get("physical_devices", []))
    bound = sorted((v["virtual_lamp_id"], (v.get("binding") or {}).get("physical_short_address"))
                   for v in lamps.get("virtual_lamps", []))
    return {"physical devices": shorts, "virtual lamps": bound}


def registry_difference(dut_cfg, peer_cfg) -> list:
    dut, peer = _registry(dut_cfg), _registry(peer_cfg)
    return ["%s: the DUT has %s, the standby %s" % (key, dut[key], peer[key])
            for key in dut if dut[key] != peer[key]]


def compare_registries(dut_cfg, peer_cfg, log=print) -> int:
    for attempt in ("", " after a reset"):
        if not _converged(dut_cfg, peer_cfg):
            print("role: the standby's slices did not match the DUT's within %ds%s"
                  % (REPLICATION_WAIT_S, attempt), file=sys.stderr)
            return 1
        difference = registry_difference(dut_cfg, peer_cfg)
        if not difference:
            log("the standby's registry matches the DUT's%s" % attempt)
            return 0
        log("the standby keeps entries its slices no longer hold: %s" % "; ".join(difference))
        if attempt:
            return 1
        remote_serial.control(peer_cfg, "run")
        if not _await_controller(peer_cfg):
            return 1
    return 1


def status(dut_cfg, peer_cfg) -> int:
    held = current(peer_cfg)
    health = controller_health(peer_cfg)
    banner = flash.ready_line(peer_cfg, 0)
    print("recorded role: %s" % json.dumps(held, sort_keys=True))
    print("peer /health:  %s" % (json.dumps(health, sort_keys=True) if health else "silent"))
    if banner:
        print("last ready line: build=%s slot=%s state=%s" % banner)
    if held.get("role") == ROLE_GEAR_SIM and health:
        print("the peer answers as a controller although the record says gear-sim: a reset "
              "handed the role back; `hil --peer role controller` records it")
    return 0
