import fcntl
import json
import os
import signal
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import numpy as np

from hil.camera.backend import CameraError
from hil.camera.backends import DirectBackend
from hil.config import load as load_config

RESTART_WAIT_S = 10.0

IDLE_KEEPALIVE_S = 300.0


def _write_heartbeat(alive, healthy, consecutive_failures, last_error):
    alive.write_text(json.dumps({
        "ts": time.time(),
        "healthy": healthy,
        "consecutive_failures": consecutive_failures,
        "last_error": last_error,
    }))


def _lockfile(cfg) -> Path:
    return Path(cfg.state_dir) / "frame_server" / "server.pid"


def acquire_singleton(cfg):
    path = _lockfile(cfg)
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = open(path, "a+")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        handle.close()
        return None
    handle.seek(0)
    handle.truncate()
    handle.write(json.dumps({"pid": os.getpid(), "started": time.time()}))
    handle.flush()
    return handle


def incumbent(cfg):
    path = _lockfile(cfg)
    if not path.exists():
        return None
    try:
        handle = open(path, "r")
    except OSError:
        return None
    with handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except OSError:
            pass
        else:
            fcntl.flock(handle, fcntl.LOCK_UN)
            return None
        try:
            body = json.loads(handle.read())
        except ValueError:
            body = {}
    started = body.get("started")
    return {
        "pid": body.get("pid"),
        "started": started,
        "uptime_s": (time.time() - started) if started else None,
    }


def describe(cfg):
    live = incumbent(cfg)
    if live is not None:
        uptime = live.get("uptime_s")
        return "frame server pid %s, up %s" % (
            live.get("pid"), _human_uptime(uptime) if uptime else "?")
    strays = stray_pids()
    if not strays:
        return "no frame server running"
    return ("frame server pid %s holding NO lock — started before the "
            "single-instance lock existed (or outside the CLI); "
            "`hil camera-server --restart` puts it under one"
            % ", ".join(str(p) for p in strays))


def _human_uptime(seconds):
    if seconds < 90:
        return "%.0fs" % seconds
    if seconds < 5400:
        return "%.1fm" % (seconds / 60.0)
    return "%.1fh" % (seconds / 3600.0)


def _log(logfile, message):
    line = "%s %s" % (time.strftime("%Y-%m-%dT%H:%M:%S"), message)
    print(line, flush=True)
    try:
        with open(logfile, "a") as fh:
            fh.write(line + "\n")
    except OSError:
        pass


def serve():
    cfg = load_config()
    mailbox = Path(cfg.state_dir) / "frame_server"
    mailbox.mkdir(parents=True, exist_ok=True)
    alive = mailbox / "server.alive"
    logfile = mailbox / "server.log"
    lock = acquire_singleton(cfg)
    if lock is None:
        _log(logfile, "frame server: REFUSING to start — %s already owns this "
                      "mailbox (%s). Use `hil camera-server --restart` to "
                      "replace it." % (describe(cfg), mailbox))
        return 1
    backend = DirectBackend(cfg)
    backend.note = lambda message: _log(logfile, "frame server: %s" % message)
    _log(logfile, "frame server: mailbox %s" % mailbox)
    _log(logfile, "frame server: first capture triggers the macOS camera prompt — click Allow")
    try:
        frame = backend.capture(warmup=5, avg=3)
    except Exception as exc:
        _log(logfile, "frame server: FIRST CAPTURE FAILED, exiting: %s" % exc)
        raise
    _log(logfile, "frame server: capture OK (mean %.1f) — serving" % float(frame.mean()))
    failures, last_error = 0, None
    last_capture = time.time()

    def recovering_capture(warmup, avg):
        nonlocal backend
        try:
            return backend.capture(warmup=warmup, avg=avg)
        except CameraError:
            backend.close()
        try:
            return backend.capture(warmup=warmup, avg=avg)
        except CameraError as exc:
            _log(logfile, "session reopen did not recover (%s) — rebuilding the backend" % exc)
            backend = DirectBackend(cfg)
            return backend.capture(warmup=max(int(warmup), 8), avg=avg)

    try:
        while True:
            _write_heartbeat(alive, failures == 0, failures, last_error)
            requests = sorted(mailbox.glob("req_*.json"))
            if not requests and time.time() - last_capture > IDLE_KEEPALIVE_S:
                try:
                    recovering_capture(warmup=2, avg=1)
                    _log(logfile, "idle keepalive: capture OK")
                except Exception as exc:
                    _log(logfile, "idle keepalive FAILED: %s" % exc)
                last_capture = time.time()
            for req_path in requests:
                rid = req_path.name[4:-5]
                try:
                    req = json.loads(req_path.read_text())
                    frame = recovering_capture(int(req.get("warmup", 4)),
                                               int(req.get("avg", 4)))
                    last_capture = time.time()
                    tmp = mailbox / (".frame_%s.%d.tmp.npz" % (rid, os.getpid()))
                    np.savez_compressed(tmp, frame=frame)
                    tmp.rename(mailbox / ("frame_%s.npz" % rid))
                    failures, last_error = 0, None
                    _log(logfile, "served frame %s (mean %.1f)" % (rid, float(frame.mean())))
                except Exception as exc:
                    failures += 1
                    last_error = str(exc)
                    (mailbox / ("frame_%s.err" % rid)).write_text(str(exc))
                    _log(logfile, "request %s failed (%d in a row): %s" % (rid, failures, exc))
                finally:
                    try:
                        req_path.unlink()
                    except OSError:
                        pass
            time.sleep(0.2)
    except BaseException as exc:
        _log(logfile, "frame server: EXITING (%s: %s)" % (type(exc).__name__, exc))
        raise


def run_in_terminal(command, ready, timeout_s=45, label="camera server"):
    hil_bin = Path(sys.executable).parent / "hil"
    script = "#!/bin/zsh\nexec %s %s\n" % (hil_bin, command)
    fd, path = tempfile.mkstemp(suffix=".command", prefix="hil_%s_" % command.split()[0])
    os.write(fd, script.encode())
    os.close(fd)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    subprocess.run(["open", "-a", "Terminal", path], check=True)
    for _ in range(timeout_s):
        if ready():
            print("%s READY" % label)
            return 0
        time.sleep(1)
    print("%s did not come up — check the Terminal window" % label, file=sys.stderr)
    return 1


def _ready(cfg):
    alive = Path(cfg.state_dir) / "frame_server" / "server.alive"
    return alive.exists() and time.time() - alive.stat().st_mtime < 5.0


def spawn_in_terminal():
    cfg = load_config()
    if incumbent(cfg) is not None or _ready(cfg):
        print("%s — nothing to do (use `--restart` to replace it)" % describe(cfg))
        return 0
    return run_in_terminal(
        "camera-server", lambda: _ready(cfg), label="camera server")


def stop(cfg, quiet=False):
    live = incumbent(cfg)
    if live is None:
        if not quiet:
            print("no frame server running")
        return 0
    pid = live.get("pid")
    try:
        os.kill(int(pid), signal.SIGTERM)
    except (OSError, TypeError, ValueError) as exc:
        print("could not signal frame server pid %s: %s" % (pid, exc),
              file=sys.stderr)
        return 1
    deadline = time.monotonic() + RESTART_WAIT_S
    while time.monotonic() < deadline:
        if incumbent(cfg) is None:
            print("frame server stopped (pid %s)" % pid)
            return 0
        time.sleep(0.2)
    print("frame server pid %s did not exit within %.0fs"
          % (pid, RESTART_WAIT_S), file=sys.stderr)
    return 1


def restart_in_terminal():
    cfg = load_config()
    if incumbent(cfg) is not None and stop(cfg) != 0:
        return 1
    return run_in_terminal(
        "camera-server", lambda: _ready(cfg), label="camera server")


def status(cfg):
    print(describe(cfg))
    mailbox = Path(cfg.state_dir) / "frame_server"
    alive = mailbox / "server.alive"
    if alive.exists():
        age = time.time() - alive.stat().st_mtime
        try:
            body = json.loads(alive.read_text())
        except (OSError, ValueError):
            body = {}
        print("heartbeat %.1fs old, healthy=%s, consecutive_failures=%s"
              % (age, body.get("healthy"), body.get("consecutive_failures")))
    strays = stray_pids()
    if len(strays) > 1:
        print("WARNING: %d `hil camera-server` processes are running (%s) — "
              "they share one mailbox and corrupt each other's frames; kill all "
              "but one" % (len(strays), ", ".join(str(p) for p in strays)),
              file=sys.stderr)
        return 1
    return 0 if incumbent(cfg) is not None else 1


def stray_pids():
    try:
        out = subprocess.run(["ps", "-Ao", "pid=,args="],
                             capture_output=True, text=True)
    except OSError:
        return []
    mine = os.getpid()
    found = []
    for line in out.stdout.splitlines():
        pid, _, args = line.strip().partition(" ")
        if not pid.isdigit() or int(pid) == mine:
            continue
        if is_serving_argv(args):
            found.append(int(pid))
    return found


def is_serving_argv(args) -> bool:
    tokens = args.split()
    for i, token in enumerate(tokens[:-1]):
        if (token.endswith("/hil") or token == "hil") \
                and tokens[i + 1] == "camera-server":
            return i + 1 == len(tokens) - 1
    return False
