import fcntl
import json
import os
import re
import subprocess
import sys
import time
import zipfile
import zlib
from pathlib import Path

import numpy as np

from hil.camera.backend import (AVG_FRAMES, WARMUP_FRAMES, CameraBackend,
                                CameraError, LockMode)
from hil.camera.capture import avg_frames, drain_to_live, open_session
from hil.camera.controls import ControlError, UvcUtilControl

BLACK_FRAME_MEAN = 2.0


def list_video_devices():
    out = subprocess.run(
        ["ffmpeg", "-f", "avfoundation", "-list_devices", "true", "-i", ""],
        capture_output=True, text=True).stderr
    devices, in_video = [], False
    for line in out.splitlines():
        if "AVFoundation video devices" in line:
            in_video = True
            continue
        if "AVFoundation audio devices" in line:
            in_video = False
            continue
        m = re.search(r"\[(\d+)\]\s+(.*)$", line)
        if in_video and m:
            devices.append((int(m.group(1)), m.group(2).strip()))
    return devices


def _sp_cameras():
    out = subprocess.run(["system_profiler", "SPCameraDataType", "-json"],
                         capture_output=True, text=True).stdout
    try:
        entries = json.loads(out).get("SPCameraDataType", [])
    except json.JSONDecodeError:
        return []
    return [{"name": e.get("_name", ""), "uid": e.get("spcamera_unique-id", ""),
             "model": e.get("spcamera_model-id", "")} for e in entries]


def _identity_path(cfg):
    return Path(cfg.state_dir) / "camera_identity.json"


def _index_for_exact_name(devices, name):
    for idx, devname in devices:
        if devname.strip() == name.strip():
            return idx
    return None


def _match_by_configured_name(devices, name):
    for idx, devname in devices:
        if name.lower() in devname.lower():
            return idx
    tokens = [t for t in name.lower().split() if len(t) >= 3 and t.isascii()]
    for idx, devname in devices:
        if any(t in devname.lower() for t in tokens):
            return idx
    return None


def resolve_index(cfg):
    devices = list_video_devices()
    sp = _sp_cameras()
    ident_file = _identity_path(cfg)
    want_uid = getattr(cfg, "camera_id", "") or ""
    if not want_uid and ident_file.exists():
        try:
            want_uid = json.loads(ident_file.read_text()).get("unique_id", "")
        except (OSError, json.JSONDecodeError):
            want_uid = ""
    if want_uid:
        cam = next((c for c in sp if c["uid"] == want_uid), None)
        if cam is not None:
            idx = _index_for_exact_name(devices, cam["name"])
            if idx is not None:
                return idx
    idx = _match_by_configured_name(devices, cfg.camera_name)
    if idx is None:
        raise CameraError("camera %r (uid %r) not found among %s"
                          % (cfg.camera_name, want_uid, devices))
    devname = dict(devices)[idx]
    cam = next((c for c in sp if c["name"].strip() == devname.strip()), None)
    if cam is not None and cam["uid"]:
        ident_file.parent.mkdir(parents=True, exist_ok=True)
        ident_file.write_text(json.dumps(
            {"unique_id": cam["uid"], "name": cam["name"],
             "model": cam["model"]}, ensure_ascii=False, indent=1))
    return idx


def _real_camera_count(devices):
    return sum(1 for _, name in devices if not name.startswith("Capture screen"))


def _responds_to_uvc(cap, controls):
    from hil.camera.backend import FLAT_RATIO

    means = {}
    for exp in (20, 400):
        controls.apply_lock(exposure=exp)
        time.sleep(0.7)
        drain_to_live(cap)
        frame = avg_frames(cap, 3)
        if frame is None:
            controls.apply_lock(release_exposure=True)
            return False
        means[exp] = float(frame.mean())
    controls.apply_lock(release_exposure=True)
    return means[400] / max(means[20], 0.1) > FLAT_RATIO


def open_bench_session(cfg, controls, note=None):
    hint = resolve_index(cfg)
    if controls is None:
        return open_session(hint)
    candidates = [hint] + [i for i in range(_real_camera_count(list_video_devices()))
                           if i != hint]
    tried = []
    for idx in candidates:
        try:
            cap = open_session(idx)
        except CameraError as exc:
            tried.append("%d: %s" % (idx, exc))
            continue
        for _ in range(25):
            cap.read()
        if _responds_to_uvc(cap, controls):
            if note and idx != hint:
                note("camera: index %d did not respond to uvc-util; serving "
                     "index %d, which does" % (hint, idx))
            return cap
        cap.release()
        tried.append("%d: opened but ignores uvc-util (not our camera)" % idx)
    raise CameraError(
        "no camera responds to uvc-util, so none of them is the bench camera "
        "(the only UVC device on this host). Tried — %s. Check the USB link "
        "speed: a hub chain that fell back to Full Speed leaves the camera "
        "unable to stream 1920x1080 at all "
        "(ioreg -p IOUSB -w0 -l | grep -E '\\+-o |Device Speed')"
        % "; ".join(tried))


class DirectBackend(CameraBackend):
    name = "direct"

    def __init__(self, cfg):
        self.cfg = cfg
        try:
            self.controls = UvcUtilControl(cfg)
        except ControlError:
            self.controls = None
        self._cap = None
        self._lock_seen = 0.0

    def _session(self):
        if self._cap is None:
            self._cap = open_bench_session(self.cfg, self.controls, note=self.note)
            self.ensure_locked()
        return self._cap

    def note(self, message):
        print(message, file=sys.stderr)

    def capture(self, warmup=WARMUP_FRAMES, avg=AVG_FRAMES):
        cap = self._session()
        if self.controls and self.controls.lock_mtime() > self._lock_seen:
            self.controls.apply_lock()
            self._lock_seen = self.controls.lock_mtime()
            time.sleep(0.3)
        drain_to_live(cap)
        for _ in range(min(int(warmup), 2)):
            cap.read()
        mean = avg_frames(cap, avg)
        if mean is None:
            self.close()
            raise CameraError("no frames from %r — TCC denies this process "
                              "(agent context) or the camera is unplugged"
                              % self.cfg.camera_name)
        if float(mean.mean()) < BLACK_FRAME_MEAN:
            self.close()
            raise CameraError("frames are black — camera permission denied or lens covered")
        return mean

    def ensure_locked(self):
        if self.controls is None:
            return LockMode.UNLOCKED
        mode = LockMode(self.controls.apply_lock())
        self._lock_seen = self.controls.lock_mtime()
        return mode

    def write_controls(self, exposure=None, wb=None):
        if self.controls is None:
            return
        self.controls.apply_lock(exposure=exposure, wb=wb,
                                 release_exposure=exposure is None)
        self._lock_seen = self.controls.lock_mtime()

    def close(self):
        if self._cap is not None:
            self._cap.release()
            self._cap = None


class FrameServerBackend(CameraBackend):
    name = "frame-server"
    TIMEOUT_S = 30.0

    def __init__(self, cfg):
        self.cfg = cfg
        self.mailbox = Path(cfg.state_dir) / "frame_server"
        try:
            self.controls = UvcUtilControl(cfg)
        except ControlError:
            self.controls = None

    def alive(self):
        alive = self.mailbox / "server.alive"
        try:
            return time.time() - alive.stat().st_mtime < 5.0
        except OSError:
            return False

    def health(self):
        try:
            body = json.loads((self.mailbox / "server.alive").read_text())
        except (OSError, ValueError):
            return None
        return body if isinstance(body, dict) and "healthy" in body else None

    def capture(self, warmup=WARMUP_FRAMES, avg=AVG_FRAMES):
        if not self.alive():
            raise CameraError("frame server not running — hil camera-server --spawn-terminal")
        health = self.health()
        if health is not None and not health.get("healthy", True):
            raise CameraError(
                "frame server is up but its last %d capture(s) failed: %s"
                % (health.get("consecutive_failures", 1), health.get("last_error")))
        self._reap_orphans()
        rid = "%d_%d" % (os.getpid(), int(time.time() * 1000))
        req = self.mailbox / ("req_%s.json" % rid)
        with open(req, "w") as fh:
            fcntl.flock(fh, fcntl.LOCK_EX)
            json.dump({"warmup": warmup, "avg": avg}, fh)
        frame_path = self.mailbox / ("frame_%s.npz" % rid)
        err_path = self.mailbox / ("frame_%s.err" % rid)
        deadline = time.monotonic() + self.TIMEOUT_S
        while time.monotonic() < deadline:
            if frame_path.exists():
                frame = self._load_frame(frame_path, rid)
                frame_path.unlink()
                return frame
            if err_path.exists():
                msg = err_path.read_text()
                err_path.unlink()
                raise CameraError("frame server: %s" % msg)
            time.sleep(0.1)
        raise CameraError(self._timeout_reason(req, err_path))

    @staticmethod
    def _load_frame(frame_path, rid):
        try:
            with np.load(frame_path) as npz:
                return npz["frame"]
        except (OSError, ValueError, EOFError, KeyError,
                zipfile.BadZipFile, zlib.error) as exc:
            frame_path.unlink(missing_ok=True)
            raise CameraError(
                "torn frame %s (%s: %s) — almost always more than one "
                "`hil camera-server` writing this mailbox; run "
                "`hil camera-server --status`" % (rid, type(exc).__name__, exc))

    def _reap_orphans(self):
        cutoff = time.time() - self.TIMEOUT_S * 4
        for pattern in ("frame_*.err", "frame_*.npz", "req_*.json"):
            for path in self.mailbox.glob(pattern):
                try:
                    if path.stat().st_mtime < cutoff:
                        path.unlink()
                except OSError:
                    pass

    def _timeout_reason(self, req, err_path):
        if err_path.exists():
            msg = err_path.read_text()
            err_path.unlink()
            return "frame server: %s" % msg
        pending = req.exists()
        try:
            req.unlink()
        except OSError:
            pass
        if pending:
            return ("frame server is heart-beating but did not pick up the "
                    "request within %.0fs — restart it: "
                    "`hil camera-server --spawn-terminal`" % self.TIMEOUT_S)
        return ("frame server took the request but did not answer within "
                "%.0fs — capture is blocked (camera unplugged, or the server "
                "process lacks the camera TCC grant). Restart it from an "
                "operator Terminal: `hil camera-server --spawn-terminal`"
                % self.TIMEOUT_S)

    def ensure_locked(self):
        if not self.alive():
            return LockMode.UNLOCKED
        if self.controls is None:
            return LockMode.UNLOCKED
        try:
            mode = json.loads(self.controls.lock_json.read_text()).get("mode")
            return LockMode(mode) if mode in ("uvc-locked", "uvc-mismatch") \
                else LockMode.UNLOCKED
        except (OSError, ValueError):
            return LockMode.UNLOCKED

    def write_controls(self, exposure=None, wb=None):
        if self.controls is None:
            return
        self.controls.apply_lock(exposure=exposure, wb=wb,
                                 release_exposure=exposure is None)
