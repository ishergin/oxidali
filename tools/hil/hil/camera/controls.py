import json
import subprocess
import time
from pathlib import Path

from hil.config import HilConfig

LOCK_DEFAULTS = {
    "auto-exposure-mode": 1,
    "exposure-time-abs": 313,
    "auto-white-balance-temp": "false",
    "white-balance-temp": 4000,
    "gain": 0,
    "power-line-frequency": 1,
    "brightness": 0,
}


class ControlError(RuntimeError):
    pass


class UvcUtilControl:
    def __init__(self, cfg: HilConfig):
        self.cfg = cfg
        self.lock_json = Path(cfg.state_dir) / "camera_lock.json"
        self._index = None
        self._implemented = None

    def _run(self, *args):
        if not self.cfg.uvc_util.exists():
            raise ControlError("uvc-util missing — run setup (vendor/uvc-util)")
        return subprocess.run([str(self.cfg.uvc_util), *args],
                              capture_output=True, text=True, timeout=10)

    def index(self):
        if self._index is None:
            out = self._run("-d").stdout
            for line in out.splitlines():
                if self.cfg.camera_name in line:
                    self._index = line.split()[0]
                    break
            else:
                raise ControlError("camera %r not on USB" % self.cfg.camera_name)
        return self._index

    def implemented(self):
        if self._implemented is None:
            out = self._run("-I", self.index(), "-c").stdout
            self._implemented = {line.strip() for line in out.splitlines()
                                 if line.startswith("  ")}
        return self._implemented

    def get(self, ctrl):
        out = self._run("-I", self.index(), "-g", ctrl).stdout.strip()
        return out.split("=")[-1].strip() if "=" in out else None

    def set(self, ctrl, value):
        return self._run("-I", self.index(), "-s", "%s=%s" % (ctrl, value)).returncode == 0

    def targets(self):
        t = dict(LOCK_DEFAULTS)
        for source in (self._stored_targets(), self._calibrated_targets()):
            t.update(source)
        if self._exposure_pinned():
            t.update(self._stored_targets())
        return t

    def _exposure_pinned(self):
        try:
            return bool(json.loads(self.lock_json.read_text()).get("exposure_pinned"))
        except (OSError, ValueError):
            return False

    def _stored_targets(self):
        try:
            stored = json.loads(self.lock_json.read_text()).get("targets", {})
        except (OSError, ValueError):
            return {}
        return {k: stored[k] for k in ("exposure-time-abs", "white-balance-temp")
                if k in stored}

    def _calibrated_targets(self):
        path = Path(self.cfg.state_dir) / "calibration.json"
        try:
            exposure = json.loads(path.read_text()).get("exposure_time_abs")
        except (OSError, ValueError):
            return {}
        return {"exposure-time-abs": int(exposure)} if exposure else {}

    def apply_lock(self, exposure=None, wb=None, release_exposure=False):
        targets = self.targets()
        pinned = self._exposure_pinned()
        if exposure is not None:
            targets["exposure-time-abs"] = int(exposure)
            pinned = True
        elif release_exposure:
            targets["exposure-time-abs"] = self._unpinned_exposure()
            pinned = False
        if wb is not None:
            targets["white-balance-temp"] = int(wb)
        implemented = self.implemented()
        mode = "uvc-locked"
        verified = {}
        for ctrl, want in targets.items():
            if ctrl not in implemented:
                verified[ctrl] = {"want": want, "got": None, "ok": True,
                                  "skipped": "not implemented by camera"}
                continue
            self.set(ctrl, want)
            got = self.get(ctrl)
            ok = str(got) == str(want)
            verified[ctrl] = {"want": want, "got": got, "ok": ok}
            if not ok:
                mode = "uvc-mismatch"
        self._persist(mode, targets, verified, pinned)
        return mode

    def _unpinned_exposure(self):
        t = dict(LOCK_DEFAULTS)
        for source in (self._stored_targets(), self._calibrated_targets()):
            t.update(source)
        return int(t["exposure-time-abs"])

    def lock_mtime(self):
        try:
            return self.lock_json.stat().st_mtime
        except OSError:
            return 0.0

    def _persist(self, mode, targets, verified, exposure_pinned=False):
        doc = {"mode": mode, "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "targets": targets, "exposure_pinned": bool(exposure_pinned)}
        if verified is not None:
            doc["verified"] = verified
        self.lock_json.parent.mkdir(parents=True, exist_ok=True)
        self.lock_json.write_text(json.dumps(doc, indent=1, sort_keys=True))
