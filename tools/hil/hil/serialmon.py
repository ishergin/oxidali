import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from hil.pidfile import pid_alive, sidecar_log


def _pidfile(cfg) -> Path:
    return Path(cfg.state_dir) / "serial_monitor.pid"


def _logfile_ref(cfg) -> Path:
    return Path(cfg.state_dir) / "serial_monitor.pid.log"


def pin_path(cfg) -> Path:
    return Path(cfg.state_dir) / "serial_port.pin"


def recorded_pin(cfg):
    try:
        port = pin_path(cfg).read_text().strip()
    except OSError:
        return None
    return port or None


def record_pin(cfg, port):
    pin_path(cfg).write_text(port + "\n")


def effective_port(cfg):
    from hil import remote_serial
    if remote_serial.enabled(cfg):
        return remote_serial.data_url(cfg), True
    if cfg.serial_port_pinned:
        return cfg.serial_port, True
    recorded = recorded_pin(cfg)
    if recorded:
        return recorded, True
    return cfg.serial_port, False


def alive(cfg):
    return pid_alive(_pidfile(cfg))


def log_size(cfg) -> int:
    log = log_path(cfg)
    try:
        return log.stat().st_size if log else 0
    except OSError:
        return 0


def announced_address(cfg, since: int = 0):
    import re
    log = log_path(cfg)
    if not log or not log.exists():
        return None
    seen = None
    with open(log, "r", errors="replace") as fh:
        if since:
            fh.seek(since)
        for line in fh:
            match = re.search(r"(?:address now|DHCP (?:already )?bound) ([0-9.]+)", line)
            if match:
                seen = match.group(1)
    return seen


def remote_enabled(cfg):
    from hil import remote_serial
    return remote_serial.enabled(cfg)


def log_path(cfg) -> Path:
    return sidecar_log(_pidfile(cfg)) or cfg.persist_serial_log


def reader_loop(port, baud, pinned=False):
    import serial

    from hil import serialport

    def ts():
        now = time.time()
        return (time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(now))
                + ".%03d" % ((now % 1) * 1000))

    while True:
        try:
            if not pinned:
                port = serialport.resolve()
            s = serial.serial_for_url(port, do_not_open=True)
            s.baudrate, s.timeout = int(baud), 1
            if "://" not in port:
                s.dtr = False
                s.rts = False
            s.open()
            sys.stderr.write("%s connected to %s @ %s\n" % (ts(), port, baud))
            sys.stderr.flush()
            buf = b""
            while True:
                chunk = s.read(4096)
                if not chunk:
                    continue
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    text = line.decode("utf-8", errors="replace").rstrip("\r")
                    sys.stdout.write("%s %s\n" % (ts(), text))
                sys.stdout.flush()
        except Exception as exc:
            sys.stderr.write("%s serial error: %s — retrying in 2s\n" % (ts(), exc))
            sys.stderr.flush()
            time.sleep(2)


def start(cfg, log=None):
    if alive(cfg):
        print("reusing running monitor (pid %s), log: %s"
              % (_pidfile(cfg).read_text().strip(), log_path(cfg)))
        return 0
    from hil import remote_serial
    if remote_serial.enabled(cfg):
        remote_serial.ensure(cfg)
    port, pinned = effective_port(cfg)
    if "://" not in port and not os.path.exists(port):
        hint = ""
        if pinned and not cfg.serial_port_pinned:
            hint = (" — pinned by %s; replug the board, set HIL_SERIAL_PORT to "
                    "the new name, or delete that file to re-enable "
                    "auto-discovery" % pin_path(cfg))
        print("serial port %s not found%s" % (port, hint), file=sys.stderr)
        return 1
    log = Path(log or cfg.persist_serial_log)
    log.parent.mkdir(parents=True, exist_ok=True)
    hil_bin = Path(sys.executable).parent / "hil"
    with open(log, "ab") as fh:
        proc = subprocess.Popen(
            [str(hil_bin), "monitor", "_run", port, str(cfg.serial_baud),
             "pinned" if pinned else "auto"],
            stdout=fh, stderr=subprocess.STDOUT, start_new_session=True)
    _pidfile(cfg).write_text(str(proc.pid))
    _logfile_ref(cfg).write_text(str(log))
    if pinned:
        record_pin(cfg, port)
    print("monitor started (pid %d, %s %s), log: %s"
          % (proc.pid, "pinned" if pinned else "auto-discovered", port, log))
    if "://" in port:
        print("NOTE: the WB bridge owns the port, so this attach did NOT reboot "
              "the DUT; it is the only serial consumer slot, and `hil flash` "
              "takes it for the duration of a flash")
    else:
        print("NOTE: attaching just rebooted the DUT (opening the port asserts "
              "reset); keep this monitor running — stopping it reboots it again")
    return 0


def stop(cfg):
    if not alive(cfg):
        print("monitor not running")
        return 0
    pid = int(_pidfile(cfg).read_text().strip())
    try:
        os.killpg(os.getpgid(pid), signal.SIGTERM)
    except OSError:
        os.kill(pid, signal.SIGTERM)
    _pidfile(cfg).unlink(missing_ok=True)
    _logfile_ref(cfg).unlink(missing_ok=True)
    print("monitor stopped (%s)"
          % ("the bridge keeps the DUT running; serial output is unrecorded "
             "until the monitor is back" if remote_enabled(cfg)
             else "the DUT reboots on port close"))
    return 0


def status(cfg):
    if alive(cfg):
        lp = log_path(cfg)
        lines = sum(1 for _ in open(lp, errors="replace")) if lp.exists() else 0
        print("running (pid %s), log: %s, lines: %d"
              % (_pidfile(cfg).read_text().strip(), lp, lines))
        return 0
    print("not running")
    return 1


def tail(cfg, n=20):
    lp = log_path(cfg)
    if not lp.exists():
        print("no log at %s" % lp, file=sys.stderr)
        return 1
    for line in open(lp, errors="replace").read().splitlines()[-n:]:
        print(line)
    return 0
