import hashlib
import socket
import subprocess
import sys
import time
from collections import namedtuple
from pathlib import Path

from hil.pidfile import pid_alive

REMOTE_DIR = "/mnt/data/dali2rust"
REMOTE_SCRIPT = REMOTE_DIR + "/serial_bridge.py"
REMOTE_LOG = REMOTE_DIR + "/serial_bridge.log"

SSH_OPTS = ("-o", "ConnectTimeout=8", "-o", "BatchMode=yes")
TUNNEL_OPTS = ("-N", "-o", "ExitOnForwardFailure=yes",
               "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3")

CONTROL_TIMEOUT_S = 6.0
BRIDGE_START_TIMEOUT_S = 10.0

PORT_GONE = "port-gone"
RESTART_RESETS = ("`hil remote start --restart` opens the port again, and opening it "
                  "resets the controller: a production event, so restarting is the "
                  "operator's decision")


class RemoteError(RuntimeError):
    pass


class BridgePortGone(RemoteError):
    pass


class Target:
    def __init__(self, spec):
        ssh, _, device = spec.rpartition(":")
        if not ssh or not device:
            raise RemoteError(
                "HIL_SERIAL_REMOTE must look like user@host:/dev/ttyACM0, got %r" % spec)
        self.ssh, self.device = ssh, device

    def __str__(self):
        return "%s:%s" % (self.ssh, self.device)


def target(cfg):
    return Target(cfg.serial_remote) if cfg.serial_remote else None


def enabled(cfg):
    return bool(cfg.serial_remote)


def data_url(cfg):
    return "rfc2217://127.0.0.1:%d" % cfg.serial_bridge_port


def control_port(cfg):
    return cfg.serial_bridge_port + 1


def tunnel_pidfile(cfg) -> Path:
    return Path(cfg.state_dir) / "serial_tunnel.pid"


def _ssh(tgt, command, timeout=20):
    return subprocess.run(
        ["ssh", *SSH_OPTS, tgt.ssh, command],
        capture_output=True, text=True, timeout=timeout)


def _local_sha(path: Path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def script_path() -> Path:
    return Path(__file__).resolve().parent.parent / "wb" / "serial_bridge.py"


def deploy(cfg, tgt):
    local = script_path()
    want = _local_sha(local)
    have = _ssh(tgt, "sha256sum %s 2>/dev/null | cut -d' ' -f1" % REMOTE_SCRIPT)
    if have.stdout.strip() == want:
        return False
    _ssh(tgt, "mkdir -p %s" % REMOTE_DIR)
    rc = subprocess.run(["scp", *SSH_OPTS, "-q", str(local),
                         "%s:%s" % (tgt.ssh, REMOTE_SCRIPT)],
                        capture_output=True, text=True, timeout=60)
    if rc.returncode != 0:
        raise RemoteError("copying the bridge to %s failed: %s"
                          % (tgt.ssh, rc.stderr.strip()))
    return True


def resolve_device(tgt):
    if "*" not in tgt.device and "?" not in tgt.device:
        return tgt.device
    out = _ssh(tgt, "ls -1 %s 2>/dev/null" % tgt.device)
    found = [line.strip() for line in out.stdout.splitlines() if line.strip()]
    if len(found) != 1:
        raise RemoteError("%s matches %d ports on %s%s"
                          % (tgt.device, len(found), tgt.ssh,
                             (": " + ", ".join(found)) if found else ""))
    tgt.device = found[0]
    return tgt.device


def bridge_listening(tgt, port):
    out = _ssh(tgt, "ss -ltn 2>/dev/null | grep -c '127.0.0.1:%d'" % port)
    return out.stdout.strip().isdigit() and int(out.stdout.strip()) > 0


KEPT_OLD_BRIDGE = ("serial bridge: a new bridge script was deployed and the running bridge "
                   "is kept, because opening the port again resets the controller; "
                   "`hil remote start --restart` applies it")


def start_bridge(cfg, tgt, restart=False):
    resolve_device(tgt)
    changed = deploy(cfg, tgt)
    if not restart and bridge_listening(tgt, cfg.serial_bridge_port):
        if changed:
            print(KEPT_OLD_BRIDGE)
        return "reusing"
    if restart or changed:
        _ssh(tgt, "fuser -k %s 2>/dev/null; fuser -k -n tcp %d %d 2>/dev/null; sleep 1"
                  % (tgt.device, cfg.serial_bridge_port, control_port(cfg)))
    _ssh(tgt, "mkdir -p %s; nohup setsid python3 %s %s %d 127.0.0.1 %d %d "
              ">> %s 2>&1 < /dev/null &"
              % (REMOTE_DIR, REMOTE_SCRIPT, tgt.device, cfg.serial_baud,
                 cfg.serial_bridge_port, control_port(cfg), REMOTE_LOG))
    deadline = time.monotonic() + BRIDGE_START_TIMEOUT_S
    while time.monotonic() < deadline:
        if bridge_listening(tgt, cfg.serial_bridge_port):
            return "started"
        time.sleep(0.5)
    log = _ssh(tgt, "tail -5 %s" % REMOTE_LOG).stdout.strip()
    raise RemoteError("bridge did not come up on %s\n%s" % (tgt, log))


TUNNEL_LIVE = "live"
TUNNEL_FOREIGN = "foreign"
TUNNEL_NONE = "none"
TUNNEL_UNASKED = "unasked"

TunnelState = namedtuple("TunnelState", "kind pid")


def _listening_pids(port):
    try:
        out = subprocess.run(["lsof", "-t", "-nP", "-iTCP:%d" % port, "-sTCP:LISTEN"],
                             capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode not in (0, 1):
        return None
    if out.returncode == 1 and out.stdout.strip():
        return None
    pids = []
    for token in out.stdout.split():
        if token.isdigit() and int(token) not in pids:
            pids.append(int(token))
    return pids


def _is_our_tunnel(pid, tgt):
    try:
        out = subprocess.run(["ps", "-o", "command=", "-p", str(pid)],
                             capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return False
    command = out.stdout.strip()
    return command.startswith("ssh") and tgt is not None and tgt.ssh in command


def tunnel_state(cfg, tgt=None) -> TunnelState:
    pids = _listening_pids(cfg.serial_bridge_port)
    if pids is None:
        pid = tunnel_pidfile(cfg).read_text().strip() if pid_alive(tunnel_pidfile(cfg)) else None
        return TunnelState(TUNNEL_UNASKED, int(pid) if pid else None)
    for pid in pids:
        if _is_our_tunnel(pid, tgt):
            return TunnelState(TUNNEL_LIVE, pid)
    if pids:
        return TunnelState(TUNNEL_FOREIGN, pids[0])
    return TunnelState(TUNNEL_NONE, None)


def tunnel_alive(cfg, tgt=None):
    state = tunnel_state(cfg, tgt)
    return state.kind in (TUNNEL_LIVE, TUNNEL_FOREIGN) or (
        state.kind == TUNNEL_UNASKED and state.pid is not None)


def start_tunnel(cfg, tgt):
    state = tunnel_state(cfg, tgt)
    if state.kind == TUNNEL_FOREIGN:
        raise RemoteError(
            "local port %d is held by pid %d, which is not an ssh to %s — the "
            "tunnel cannot bind. Free the port or point HIL_SERIAL_BRIDGE_PORT "
            "elsewhere." % (cfg.serial_bridge_port, state.pid, tgt.ssh))
    if state.pid is not None:
        tunnel_pidfile(cfg).write_text(str(state.pid))
        return "reusing"
    if state.kind == TUNNEL_UNASKED:
        pass
    log = Path(cfg.state_dir) / "serial_tunnel.log"
    with open(log, "ab") as fh:
        proc = subprocess.Popen(
            ["ssh", *SSH_OPTS, *TUNNEL_OPTS,
             "-L", "%d:127.0.0.1:%d" % (cfg.serial_bridge_port, cfg.serial_bridge_port),
             "-L", "%d:127.0.0.1:%d" % (control_port(cfg), control_port(cfg)),
             tgt.ssh],
            stdout=fh, stderr=subprocess.STDOUT, start_new_session=True)
    tunnel_pidfile(cfg).write_text(str(proc.pid))
    return "started"


def stop_tunnel(cfg, tgt=None):
    state = tunnel_state(cfg, tgt)
    if state.kind == TUNNEL_FOREIGN:
        print("local port %d is held by pid %d, which is not our tunnel — not killing it"
              % (cfg.serial_bridge_port, state.pid), file=sys.stderr)
        return False
    if state.pid is None:
        tunnel_pidfile(cfg).unlink(missing_ok=True)
        return False
    import os
    import signal
    try:
        os.killpg(os.getpgid(state.pid), signal.SIGTERM)
    except OSError:
        try:
            os.kill(state.pid, signal.SIGTERM)
        except OSError:
            tunnel_pidfile(cfg).unlink(missing_ok=True)
            return False
    tunnel_pidfile(cfg).unlink(missing_ok=True)
    return True


def control(cfg, command, timeout=CONTROL_TIMEOUT_S):
    with socket.create_connection(("127.0.0.1", control_port(cfg)), timeout) as sock:
        sock.sendall(command.encode("ascii"))
        reply = sock.recv(256).decode("ascii", "replace").strip()
    if reply.startswith("ok"):
        return reply
    if reply.startswith("err %s" % PORT_GONE):
        raise BridgePortGone("the serial bridge answers %r but its serial port is gone, "
                             "so it carries no data (%s); %s"
                             % (command, reply, RESTART_RESETS))
    raise RemoteError("bridge refused %r: %s" % (command, reply))


def ensure(cfg, restart=False, verbose=True):
    tgt = target(cfg)
    if tgt is None:
        return None
    bridge = start_bridge(cfg, tgt, restart=restart)
    if restart:
        stop_tunnel(cfg, tgt)
    tunnel = start_tunnel(cfg, tgt)
    deadline = time.monotonic() + BRIDGE_START_TIMEOUT_S
    last = None
    while time.monotonic() < deadline:
        try:
            reply = control(cfg, "ping")
            if verbose:
                print("serial bridge: %s (%s), tunnel %s — %s"
                      % (tgt, bridge, tunnel, reply), flush=True)
            return reply
        except BridgePortGone:
            raise
        except (OSError, RemoteError) as exc:
            last = exc
            time.sleep(0.5)
    raise RemoteError("bridge unreachable through the tunnel: %s" % last)


def _tunnel_description(cfg, tgt):
    state = tunnel_state(cfg, tgt)
    if state.kind == TUNNEL_LIVE:
        return "running (pid %d)" % state.pid
    if state.kind == TUNNEL_FOREIGN:
        return ("port %d held by pid %d, which is NOT an ssh to %s"
                % (cfg.serial_bridge_port, state.pid, tgt.ssh))
    if state.kind == TUNNEL_UNASKED:
        return ("no lsof here — by pidfile only: %s"
                % ("running (pid %d)" % state.pid if state.pid else "not running"))
    return "not running"


def status(cfg):
    tgt = target(cfg)
    if tgt is None:
        print("serial is local (HIL_SERIAL_REMOTE unset)")
        return 0
    print("remote DUT:  %s" % tgt)
    print("data URL:    %s" % data_url(cfg))
    print("tunnel:      %s" % _tunnel_description(cfg, tgt))
    try:
        print("bridge:      %s" % control(cfg, "status"))
        return 0
    except BridgePortGone as exc:
        print("bridge:      %s" % exc, file=sys.stderr)
        return 1
    except (OSError, RemoteError) as exc:
        print("bridge:      unreachable (%s)" % exc, file=sys.stderr)
        return 1
