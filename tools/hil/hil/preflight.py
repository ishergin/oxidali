import json
import os
import shutil
import subprocess
import time
from pathlib import Path

FRESH_HEARTBEAT_S = 5.0
OK, WARN, FAIL = "OK", "WARN", "FAIL"


def _check_cwd(cfg):
    if Path.cwd().resolve() == Path(cfg.root).resolve():
        return OK, "cwd", "tools/hil (pytest testpaths are relative)"
    return WARN, "cwd", ("%s — pytest must run from %s (`cd tools/hil`)"
                         % (Path.cwd(), cfg.root))


def _announced_address(cfg):
    from hil import serialmon
    return serialmon.announced_address(cfg)


def _check_dut(cfg):
    from hil.api import Client
    try:
        health = Client(cfg).health()
    except Exception as exc:
        announced = _announced_address(cfg)
        hint = ""
        if announced and announced not in cfg.base:
            hint = " — serial log says the board is on %s; set HIL_BASE=http://%s" % (
                announced, announced)
        return FAIL, "DUT", "%s unreachable: %s%s" % (cfg.base, exc, hint)
    return OK, "DUT", "%s (version=%s uptime=%ss)" % (
        cfg.base, health.get("version"), health.get("uptime_seconds"))


def _check_serial_port(cfg):
    from hil import remote_serial, serialmon, serialport
    if remote_serial.enabled(cfg):
        try:
            reply = remote_serial.control(cfg, "status")
        except remote_serial.BridgePortGone as exc:
            return FAIL, "serial port", "%s via %s — %s" % (
                remote_serial.data_url(cfg), remote_serial.target(cfg), exc)
        except (OSError, remote_serial.RemoteError) as exc:
            return FAIL, "serial port", (
                "%s via %s — bridge unreachable (%s); `hil remote start`"
                % (remote_serial.data_url(cfg), remote_serial.target(cfg), exc))
        return OK, "serial port", "%s via %s — %s" % (
            remote_serial.data_url(cfg), remote_serial.target(cfg), reply)
    port, pinned = serialmon.effective_port(cfg)
    detail = serialport.describe(cfg, recorded=serialmon.recorded_pin(cfg))
    if not os.path.exists(port):
        return FAIL, "serial port", detail + " — not present"
    if not pinned and len(serialport.candidates()) > 1:
        return WARN, "serial port", detail
    return OK, "serial port", detail


def _check_gear_sim(cfg):
    from hil import serialmon
    if not cfg.gear_sim_port:
        return OK, "gear sim", "not configured (set HIL_GEAR_SIM_PORT to use one)"
    if cfg.gear_sim_port == serialmon.effective_port(cfg)[0]:
        return FAIL, "gear sim", (
            "%s is also the DUT's port — HIL_GEAR_SIM_PORT must name the C6"
            % cfg.gear_sim_port)
    if not os.path.exists(cfg.gear_sim_port):
        return FAIL, "gear sim", "%s not present" % cfg.gear_sim_port
    return OK, "gear sim", "%s" % cfg.gear_sim_port


def _check_monitor(cfg):
    from hil import serialmon
    if not serialmon.alive(cfg):
        return FAIL, "serial monitor", (
            "not running — `hil monitor start` (keep it persistent: "
            "opening the port reboots the DUT)")

    log = serialmon.log_path(cfg)
    before = log.stat().st_size if log.exists() else -1
    try:
        from hil.api import Client
        Client(cfg).health()
    except Exception:
        return WARN, "serial monitor", (
            "%s — alive, but the DUT did not answer so the channel was not "
            "exercised" % log)

    deadline = time.monotonic() + 4.0
    while time.monotonic() < deadline:
        if log.exists() and log.stat().st_size > before:
            return OK, "serial monitor", str(log)
        time.sleep(0.25)
    return FAIL, "serial monitor", (
        "%s — process alive but the channel is SILENT: the DUT answered HTTP "
        "and still nothing reached the log. The /dev node is usually stale after "
        "a host reboot or re-enumeration — re-plug the DUT's USB cable, then "
        "`hil monitor stop && hil monitor start`" % log)


def _check_wb(cfg):
    from hil.sniffer import ssh_argv
    try:
        probe = subprocess.run(ssh_argv(cfg, "command -v mosquitto_sub"),
                               capture_output=True, timeout=15)
    except subprocess.TimeoutExpired:
        return FAIL, "WB sniffer host", "%s: ssh timed out" % cfg.wb_ssh
    if probe.returncode != 0:
        return FAIL, "WB sniffer host", (
            "%s unreachable over ssh (BatchMode)" % cfg.wb_ssh)
    if not probe.stdout.strip():
        return WARN, "WB sniffer host", "ssh OK but mosquitto_sub missing"
    return OK, "WB sniffer host", "%s (mosquitto_sub present)" % cfg.wb_ssh


def _check_mqtt_broker(cfg):
    from hil.sniffer import ssh_argv
    cmd = "mosquitto_sub -C 1 -W 2 -t '$SYS/broker/version'"
    try:
        probe = subprocess.run(ssh_argv(cfg, cmd), capture_output=True, timeout=15)
    except subprocess.TimeoutExpired:
        return WARN, "MQTT broker", "ssh to %s timed out" % cfg.wb_ssh
    version = probe.stdout.decode("utf-8", "replace").strip()
    if probe.returncode != 0 or not version:
        return WARN, "MQTT broker", (
            "no $SYS answer on %s:%d — ha_bridge tests will not connect"
            % (cfg.mqtt_broker_host, cfg.mqtt_broker_port))
    return OK, "MQTT broker", "%s:%d (%s)" % (
        cfg.mqtt_broker_host, cfg.mqtt_broker_port, version)


def _check_camera(cfg):
    from hil.camera import server as server_mod

    strays = server_mod.stray_pids()
    if len(strays) > 1:
        return FAIL, "camera", (
            "%d `hil camera-server` processes are running (%s) — they share one "
            "mailbox and corrupt each other's frames (Bad CRC-32 / invalid "
            "block type in unrelated tests). Kill all but one, or "
            "`hil camera-server --restart`"
            % (len(strays), ", ".join(str(p) for p in strays)))
    alive = Path(cfg.state_dir) / "frame_server" / "server.alive"
    if not alive.exists():
        return WARN, "camera", (
            "no frame server; direct capture works only from an operator "
            "Terminal (TCC). Agent runs: `hil camera-server --spawn-terminal`. "
            "Without a camera, optical tests FAIL unless --no-camera/"
            "HIL_NO_CAMERA=1")
    age = time.time() - alive.stat().st_mtime
    if age >= FRESH_HEARTBEAT_S:
        return WARN, "camera", (
            "frame server heartbeat is stale (%.0fs) — restart it: "
            "`hil camera-server --restart`" % age)
    return _probe_frame(cfg, age)


def _probe_frame(cfg, age):
    from hil.camera.backend import CameraError, probe_and_select

    backend = None
    try:
        backend = probe_and_select(cfg)
        frame = backend.capture(warmup=2, avg=1)
    except CameraError as exc:
        return FAIL, "camera", "%s [exposure=%s x100us]" % (
            exc, _effective_exposure(cfg))
    finally:
        if backend is not None:
            backend.close()
    from hil.camera import server as server_mod

    status, detail = _check_stream_is_the_bench_camera(cfg)
    return status, "camera", "frame via %s: mean=%.1f, %s, exposure=%s x100us%s" % (
        backend.name, float(frame.mean()), server_mod.describe(cfg),
        _effective_exposure(cfg), detail)


def _check_stream_is_the_bench_camera(cfg):
    from hil.camera.backend import (CameraError, FLAT_RATIO, probe_and_select)
    from hil.camera.capture import BAND_MIN_PX

    backend = None
    try:
        backend = probe_and_select(cfg)
        probe = backend.effect_probe()
    except CameraError as exc:
        return WARN, " — identity UNPROVEN (%s)" % exc
    finally:
        if backend is not None:
            backend.close()
    if probe.band_px < BAND_MIN_PX:
        return WARN, (" — identity UNPROVEN: only %d mid-band px, too dark or "
                      "too clipped for a 10x exposure command to show"
                      % probe.band_px)
    if probe.ratio < FLAT_RATIO:
        return FAIL, (
            " — THE SERVED STREAM IS NOT THE BENCH CAMERA: a 10x exposure "
            "command moved it %.2fx over %d mid-band px. uvc-util is driving "
            "one camera and the server is capturing another. Replug the USB "
            "camera (different port, no hub), then `hil camera-server "
            "--restart`" % (probe.ratio, probe.band_px))
    return OK, " — identity OK (%.2fx exposure response)" % probe.ratio


def _effective_exposure(cfg):
    from hil.camera.controls import UvcUtilControl

    try:
        return UvcUtilControl(cfg).targets().get("exposure-time-abs", "?")
    except Exception:
        return "?"


def _check_calibration(cfg):
    from hil.camera.calibrate import CALIBRATION_TTL_S, SCHEMA_VERSION, at_exposure_floor

    path = Path(cfg.state_dir) / "calibration.json"
    if not path.exists():
        return WARN, "calibration", "missing — run `hil calibrate`"
    age_h = (time.time() - path.stat().st_mtime) / 3600.0
    try:
        doc = json.loads(path.read_text())
    except (OSError, ValueError):
        return WARN, "calibration", "%s unreadable" % path
    profile = doc.get("profile")
    if doc.get("version") != SCHEMA_VERSION:
        return WARN, "calibration", (
            "profile=%s, schema v%s — this build needs v%s: run `hil calibrate` "
            "(v3 refuses to record a colour fingerprint the camera could not "
            "actually measure; a v2 file may hold blind ones)"
            % (profile, doc.get("version", 1), SCHEMA_VERSION))
    if at_exposure_floor(doc):
        return WARN, "calibration", (
            "profile=%s, %.1fh old — tuned down to the exposure FLOOR "
            "(%s x100us) with the ring still clipping at level 254: usable "
            "(tests measure at 120-140) but there is no headroom left. Dim the "
            "rig or move the camera — recalibration alone cannot buy headroom, "
            "which is why a session re-measuring does not clear this"
            % (profile, age_h, doc.get("exposure_time_abs")))
    return OK, "calibration", (
        "profile=%s, %.1fh old on disk — a session re-measures before its first "
        "optical test and whenever the profile ages past %.0f min mid-run; this "
        "matters only for `--skip-calibration`"
        % (profile, age_h, CALIBRATION_TTL_S / 60.0))


def _check_host_tools(cfg):
    missing = [name for name, present in
               (("ffmpeg", shutil.which("ffmpeg") is not None),
                ("uvc-util", cfg.uvc_util.exists())) if not present]
    if missing:
        return WARN, "host tools", "missing: %s (setup.sh builds uvc-util)" \
            % ", ".join(missing)
    return OK, "host tools", "ffmpeg + uvc-util present"


CHECKS = (_check_cwd, _check_dut, _check_serial_port, _check_gear_sim, _check_monitor, _check_wb,
          _check_mqtt_broker, _check_camera, _check_calibration, _check_host_tools)


def run(cfg):
    results = [check(cfg) for check in CHECKS]
    for status, label, detail in results:
        print("%4s  %-15s %s" % (status, label, detail))
    counts = {s: sum(1 for r in results if r[0] == s) for s in (OK, WARN, FAIL)}
    print("preflight: %d ok, %d warn, %d fail"
          % (counts[OK], counts[WARN], counts[FAIL]))
    return 1 if counts[FAIL] else 0
