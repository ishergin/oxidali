import json
import sys
import time
from pathlib import Path

import requests

from hil import flash, remote_serial, serialmon

ROLE_PIN = "role.pin"
ROLE_CONTROLLER = "controller"
ROLE_GEAR_SIM = "gear-sim"
ROLE_RETURN_TIMEOUT_S = 120
ROLE_POLL_S = 2.0
REQUEST_TIMEOUT_S = 3
LEDGER = "virtual_gear.json"


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


def preconditions(dut_cfg, peer_cfg) -> list:
    problems = []
    dut = _get(dut_cfg.base, "/api/v1/health")
    peer = _get(peer_cfg.base, "/api/v1/health")
    if not dut or dut.get("role") != "active":
        problems.append("the DUT (%s) is not active: %r" % (dut_cfg.base, dut))
    if not peer or peer.get("role") != "standby":
        problems.append("the peer (%s) is not a standby controller: %r" % (peer_cfg.base, peer))
    if (Path(dut_cfg.state_dir) / LEDGER).exists():
        problems.append("a virtual-gear session left %s: finish its teardown first"
                        % (Path(dut_cfg.state_dir) / LEDGER))
    if remote_serial.enabled(peer_cfg) and not serialmon.alive(peer_cfg):
        problems.append("the peer's serial monitor is not running: start it with "
                        "`hil --peer monitor start` while the peer is still a controller "
                        "(starting a bridge that is down resets the board)")
    return problems


def to_gear_sim(dut_cfg, peer_cfg, via=flash.VIA_OTA, log=print) -> int:
    if is_gear_sim(peer_cfg):
        log("the peer already runs the gear emulator (%s)" % current(peer_cfg))
        return 0
    problems = preconditions(dut_cfg, peer_cfg)
    if problems:
        for problem in problems:
            print("role: %s" % problem, file=sys.stderr)
        return 2
    rc = flash.run(peer_cfg, image=flash.GEAR_SIM, via=via)
    if rc != 0:
        return rc
    _record(peer_cfg, ROLE_GEAR_SIM, via=via)
    log("the peer runs the gear emulator (%s); `hil --peer role controller` hands it back"
        % via)
    return 0


def to_controller(dut_cfg, peer_cfg, log=print) -> int:
    held = current(peer_cfg)
    if held.get("role") != ROLE_GEAR_SIM:
        log("the peer already runs the controller")
        return 0
    if (Path(dut_cfg.state_dir) / LEDGER).exists():
        print("role: a virtual-gear session is still open (%s): its teardown must "
              "clean the DUT's registry before the standby pulls it"
              % (Path(dut_cfg.state_dir) / LEDGER), file=sys.stderr)
        return 2
    if held.get("via") == flash.VIA_OTA:
        remote_serial.control(peer_cfg, "write reboot")
        rc = 0 if _await_controller(peer_cfg) else 1
    else:
        rc = flash.run(peer_cfg, image=flash.CONTROLLER, via=flash.VIA_WB)
    if rc == 0:
        _record(peer_cfg, ROLE_CONTROLLER)
        log("the peer runs the controller again")
    return rc


def _await_controller(peer_cfg, timeout_s=ROLE_RETURN_TIMEOUT_S) -> bool:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        health = _get(peer_cfg.base, "/api/v1/health")
        if health and health.get("role") == "standby":
            return True
        time.sleep(ROLE_POLL_S)
    print("role: the peer did not answer as a standby controller within %ds"
          % timeout_s, file=sys.stderr)
    return False


def status(dut_cfg, peer_cfg) -> int:
    held = current(peer_cfg)
    health = _get(peer_cfg.base, "/api/v1/health") if peer_cfg.base else None
    banner = flash.ready_line(peer_cfg, 0) if held.get("role") == ROLE_GEAR_SIM else None
    print("recorded role: %s" % json.dumps(held, sort_keys=True))
    print("peer /health:  %s" % (json.dumps(health, sort_keys=True) if health else "silent"))
    if banner:
        print("last ready line: build=%s slot=%s state=%s" % banner)
    return 0
