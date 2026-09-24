import json
import os
import time
from pathlib import Path

import numpy as np

from hil import identity as identity_mod
from hil.api import ApiError, CapabilityUnsupported
from hil.camera import masks as M
from hil.camera import metrics as MX
from hil.camera.backend import FLAT_RATIO, LINEAR_RATIO, Capability
from hil.results import dumps

SETTLE_S = 1.6
COLOR_RAMP_S = 2.0
SOLO_LEVEL = 120
CLIP_TARGET = 0.05
MIN_EXPOSURE = 8
DEFAULT_EXPOSURE = 313
LADDER = (60, 100, 140, 180, 220, 254)
NIGHT_BASELINE_MEAN = 40.0
MONO_METRIC = "diff_sum_lum"

SCHEMA_VERSION = 3
HUE_OK = {"red": lambda h: h >= 330 or h <= 30, "green": lambda h: 90 <= h <= 170,
          "blue": lambda h: 190 <= h <= 260, "white": lambda h: True}
WHITE_MAX_SAT = 0.75
RGB_SETPOINTS = (("red", (254, 0, 0)), ("green", (0, 254, 0)),
                 ("blue", (0, 0, 254)), ("white", (254, 254, 254)))
RGB_PRIMARIES = RGB_SETPOINTS[:3]
CCT_SETPOINTS = (2700, 4000, 6500)
CCT_TEST_LEVEL = 140


def at_exposure_floor(doc):
    if doc.get("exposure_floor_hit"):
        return True
    exposure = doc.get("exposure_time_abs")
    return isinstance(exposure, int) and exposure <= MIN_EXPOSURE


def _why_rejected(m):
    return (m["colour_blind"] if m["colour_blind"] is not None
            else "hue=%s sat=%s off-window" % (m["hue"], m["sat"]))


def colour_ok(name, m):
    if m["colour_blind"] is not None:
        return False
    if name == "white":
        return m["sat"] <= WHITE_MAX_SAT
    return HUE_OK[name](m["hue"])


def rgb_setpoint(rgb, level=200):
    return {"power": "on", "level": level, "color_mode": "rgb",
            "rgb": {"r": rgb[0], "g": rgb[1], "b": rgb[2]}}


def cct_setpoint(kelvin, level=CCT_TEST_LEVEL):
    return {"power": "on", "level": level, "color_mode": "cct",
            "color_temperature_kelvin": kelvin}


def pick_cct_anchor(candidates, exclude, is_rgb):
    pool = [c for c in candidates if c != exclude]
    return next((c for c in pool if not is_rgb(c)), pool[0] if pool else None)


class CalibrationError(RuntimeError):
    pass


def active_path(cfg) -> Path:
    return Path(cfg.state_dir) / "calibration.json"


CALIBRATION_TTL_S = int(os.environ.get("HIL_CALIBRATION_TTL_S", "900"))


def age_seconds(doc, now=None):
    created = doc.get("created")
    if not created:
        return float("inf")
    try:
        made = time.mktime(time.strptime(created, "%Y-%m-%dT%H:%M:%S"))
    except (ValueError, TypeError):
        return float("inf")
    return max(0.0, (now if now is not None else time.time()) - made)


def is_stale(doc, ttl_s=CALIBRATION_TTL_S, now=None):
    return age_seconds(doc, now) > ttl_s


def load(cfg):
    path = active_path(cfg)
    if not path.exists():
        raise CalibrationError("calibration missing — run: hil calibrate")
    doc = json.loads(path.read_text())
    if doc.get("version") != SCHEMA_VERSION:
        raise CalibrationError(
            "calibration schema v%s is not supported — run: hil calibrate"
            % doc.get("version", 1))
    return doc


def _flushing_log(*args):
    print(*args, flush=True)


class Calibrator:
    def __init__(self, cfg, api, backend, log=_flushing_log):
        self.cfg = cfg
        self.api = api
        self.backend = backend
        self.log = log

    def _capture(self, settle=SETTLE_S):
        time.sleep(settle)
        return self.backend.capture()

    def _on(self, short, level=SOLO_LEVEL):
        self.api.ts(short, {"power": "on", "level": level})

    def _off(self, short):
        self.api.off(short)

    def _caps_primed(self, addrs):
        def read():
            return {d["short_address"]: d.get("capabilities", {})
                    for d in self.api.devices()["physical_devices"]}
        caps = read()
        if any(c.get("rgb") or c.get("cct") for c in caps.values()):
            return caps
        self.log("colour capabilities unknown (post-reboot?) — priming attr-reads...")
        for a in addrs:
            try:
                op = self.api.attr_read(a)
                self.api.wait_op(op["operation_id"])
            except (ApiError, CapabilityUnsupported) as exc:
                self.log("  prime addr %d failed: %s" % (a, exc))
        return read()

    def _solo_masks(self, addrs, base):
        geo = {}
        for addr in addrs:
            self.log("mask for addr %d @ level %d..." % (addr, SOLO_LEVEL))
            mask = None
            for attempt in (1, 2):
                pre = self.backend.capture()
                self._on(addr)
                frame = self._capture()
                self._off(addr)
                diff = M.gray_absdiff(frame, pre)
                mask = M.detect_mask(diff)
                if mask is not None:
                    break
                self.log("  attempt %d: nothing lit up, retrying" % attempt)
                time.sleep(1.0)
            if mask is None:
                self.log("WARNING: no mask for addr %d (dead or out of frame)" % addr)
                continue
            geo[addr] = {"mask": mask, "core": M.core_from(mask, diff),
                         "ring": M.ring_from(mask)}
            self.log("  mask px=%d core px=%d" % (mask.sum(), geo[addr]["core"].sum()))
        return geo

    def _solo_capture(self, addrs, geo):
        frames = {}
        for addr in addrs:
            if addr not in geo:
                continue
            pre = self.backend.capture()
            self._on(addr)
            frame = self._capture()
            self._off(addr)
            frames[addr] = frame
            diff = M.gray_absdiff(frame, pre)
            if float(diff[geo[addr]["mask"]].max() or 0) > 25.0:
                geo[addr]["core"] = M.core_from(geo[addr]["mask"], diff)
                self.log("  re-aimed core addr %d: %d px"
                         % (addr, geo[addr]["core"].sum()))
        return frames

    def _crosstalk_thresholds(self, geo, solo_frames, base):
        on_delta = {}
        for a, g in geo.items():
            def focal(frame):
                d = np.abs(frame.max(axis=2) - base.max(axis=2))
                return float(d[g["core"]].mean() - d[g["ring"]].mean())
            own = focal(solo_frames[a])
            spill = max((focal(solo_frames[b]) for b in geo if b != a), default=0.0)
            thr = max(8.0, 2.0 * max(spill, 0.0))
            if thr >= own * 0.7:
                thr = max(8.0, own * 0.5)
                self.log("  WARNING addr %s: focal spill %.1f close to own %.1f -> thr %.1f"
                         % (a, spill, own, thr))
            on_delta[a] = round(thr, 1)
            self.log("  focal addr %s: own=%.1f spill=%.1f on_delta=%.1f"
                     % (a, own, spill, on_delta[a]))
        return on_delta

    def _tune_exposure(self, addrs, geo):
        exposure = DEFAULT_EXPOSURE
        self.backend.write_controls(exposure=exposure)
        for a in addrs:
            self._on(a, 254)
        time.sleep(SETTLE_S)
        self.log("exposure tuning vs GLOW-RING clipping (all on @254)...")
        clips = {}
        for _ in range(10):
            frame = self.backend.capture()
            clips = {a: round(MX.ring_clip_fraction(frame, g["core"], g["ring"]), 4)
                     for a, g in geo.items()}
            worst = max(clips.values()) if clips else 0.0
            self.log("  exposure=%d worst ring-clip=%.3f %s" % (exposure, worst, clips))
            if worst <= CLIP_TARGET or exposure <= MIN_EXPOSURE:
                break
            exposure = max(MIN_EXPOSURE, int(exposure * 0.55))
            self.backend.write_controls(exposure=exposure)
            time.sleep(0.5)
        self.api.off_many(addrs)
        floor_hit = exposure <= MIN_EXPOSURE and (max(clips.values()) if clips else 0.0) > CLIP_TARGET
        if floor_hit:
            self.log("  WARNING: exposure hit the %d floor with ring-clip still "
                     "above %.3f — dim the rig or move the camera" % (MIN_EXPOSURE, CLIP_TARGET))
        return exposure, clips, floor_hit

    def _noise_floors(self, geo, base1, base2):
        floors = {}
        for a, g in geo.items():
            m1 = MX.lamp_metrics(base1, g["mask"])
            m2 = MX.lamp_metrics(base2, g["mask"])
            floors[a] = {
                "noise_sum_lum": round(abs(m1["sum_lum"] - m2["sum_lum"]) + 1e-3, 3),
            }
        return floors

    def _ladder(self, addr, geo, base, floors):
        self.log("SNR level ladder on addr %s (%s)..." % (addr, MONO_METRIC))
        g = geo[addr]
        ladder = {}
        for level in LADDER:
            self._on(addr, level)
            frame = self._capture()
            m = MX.lamp_metrics(frame, g["mask"], g["core"], g["ring"],
                                baseline_bgr=base)
            ladder[level] = m[MONO_METRIC]
            self.log("  level %d -> %.1f" % (level, ladder[level]))
        self._off(addr)
        span = max(ladder.values()) - min(ladder.values())
        sep = max(0.08 * span, 4.0 * floors[addr]["noise_sum_lum"])
        monotonic = []
        for level in LADDER:
            if not monotonic or ladder[level] - ladder[monotonic[-1]] > sep:
                monotonic.append(level)
        if len(monotonic) < 3:
            monotonic = [100, 170, 254]
        self.log("monotonic levels: %s (separation > %.1f)" % (monotonic, sep))
        return ladder, monotonic

    def _rgb_fingerprints(self, rgb_addr, geo, base):
        out = {}
        for name, rgb in RGB_SETPOINTS:
            m = None
            for attempt in (1, 2):
                try:
                    self.api.ts(rgb_addr, rgb_setpoint(rgb))
                except CapabilityUnsupported:
                    self.log("  rgb %s: unsupported on this firmware — skipped" % name)
                    m = None
                    break
                time.sleep(COLOR_RAMP_S)
                frame = self._capture()
                g = geo[rgb_addr]
                m = MX.lamp_metrics(frame, g["mask"], g["core"], g["ring"],
                                    baseline_bgr=base)
                if colour_ok(name, m):
                    break
                self.log("  rgb %s: rejected (%s) (attempt %d) — retrying"
                         % (name, _why_rejected(m), attempt))
            if m is None:
                continue
            if not colour_ok(name, m):
                self.log("  rgb %s: NO fingerprint — %s" % (name, _why_rejected(m)))
                continue
            out[name] = {"mean_rgb": m["mean_rgb"], "xy": m["xy"], "hue": m["hue"]}
            self.log("  rgb %s: hue=%s rgb=%s" % (name, m["hue"], m["mean_rgb"]))
        self._off(rgb_addr)
        return out

    def _cct_fingerprints(self, cct_addr, anchor, geo, base):
        out = {}
        if anchor is not None:
            self._on(anchor, 120)
        for kelvin in CCT_SETPOINTS:
            try:
                self.api.ts(cct_addr, cct_setpoint(kelvin))
            except CapabilityUnsupported:
                self.log("  cct %dK: unsupported — skipped" % kelvin)
                continue
            time.sleep(COLOR_RAMP_S)
            frame = self._capture()
            g = geo[cct_addr]
            m = MX.lamp_metrics(frame, g["mask"], g["core"], g["ring"],
                                baseline_bgr=base)
            if m["colour_blind"] is not None:
                self.log("  cct %dK: NO fingerprint — %s (%.0f%% of the colour "
                         "region clipped)"
                         % (kelvin, m["colour_blind"],
                            100.0 * m["colour_clip_fraction"]))
                continue
            out[str(kelvin)] = {"rb_ratio": m["rb_ratio"], "xy": m["xy"]}
            self.log("  cct %dK: rb_ratio=%.3f" % (kelvin, m["rb_ratio"]))
        self._off(cct_addr)
        if anchor is not None:
            self._off(anchor)
        return out

    def _fingerprints(self, addrs, caps, geo, base):
        fp = {}
        rgb_addr = next((a for a in addrs if caps.get(a, {}).get("rgb")), None)
        if rgb_addr is not None and rgb_addr in geo:
            rgb_fp = self._rgb_fingerprints(rgb_addr, geo, base)
            if rgb_fp:
                fp["rgb_addr_%d" % rgb_addr] = rgb_fp
        cct_addr = next((a for a in addrs if caps.get(a, {}).get("cct")
                         and not caps.get(a, {}).get("rgb")),
                        next((a for a in addrs if caps.get(a, {}).get("cct")), None))
        if cct_addr is not None and cct_addr in geo:
            anchor = pick_cct_anchor([a for a in addrs if a in geo], cct_addr,
                                     lambda a: caps.get(a, {}).get("rgb"))
            cct_fp = self._cct_fingerprints(cct_addr, anchor, geo, base)
            if cct_fp:
                fp["cct_addr_%d" % cct_addr] = cct_fp
            fp["cct_anchor"] = {"addr": anchor}
        return fp

    def _require_photometric(self, addrs):
        lock_mode = self.backend.ensure_locked()
        self.log("camera lock: %s (backend %s)" % (lock_mode.value, self.backend.name))
        probe = self.backend.effect_probe(
            light=lambda on: (self._on(addrs[0], 180) if on else self._off(addrs[0])))
        self.log("effect probe: ratio=%.2f capability=%s"
                 % (probe.ratio, probe.capability.value))
        if probe.ratio < FLAT_RATIO:
            raise CalibrationError(
                "a 10x exposure command moved the image %.2fx over %d mid-band "
                "px — the stream does not respond to uvc-util AT ALL, so it is "
                "not the camera being driven (or it is frozen). No lighting "
                "produces this; do not chase the scene. Replug the USB camera "
                "(different port, no hub) and `hil camera-server --restart`, "
                "then confirm it offers the full mode list: ffmpeg -f "
                "avfoundation -video_size 1920x1080 -i <n> -frames:v 1 -y "
                "/tmp/probe.png" % (probe.ratio, probe.band_px))
        if probe.capability is not Capability.PHOTOMETRIC:
            raise CalibrationError(
                "camera does not honor manual exposure (10x command -> %.1fx "
                "linear response) — HIL requires a photometric camera. Most "
                "likely the scene is too bright for the mid band: check that no "
                "lamp or room light is flooding it, then the lock "
                "(hil camera-bench --spawn-terminal); replace the camera only "
                "if a dark scene still measures below %.1fx"
                % (probe.ratio, LINEAR_RATIO))
        return lock_mode, probe

    def _infer_profile(self, base1):
        return "night" if float(base1.mean()) < NIGHT_BASELINE_MEAN else "day"

    def _lamp_records(self, geo, identities, on_delta, floors, final_clips):
        ident_by_addr = {i.short_address: i for i in identities}
        lamps = []
        for addr in geo:
            ident = ident_by_addr.get(addr)
            lamps.append({
                "identity": {"gtin": ident.gtin if ident else None,
                             "identification_number":
                                 ident.identification_number if ident else None},
                "short_address": addr,
                "label": addr + 1,
                "thresholds": {"on_delta": on_delta[addr], **floors[addr]},
                "clip_fraction_at_254": final_clips.get(addr),
            })
        return lamps

    def _save_masks(self, geo, base1):
        masks_dir = Path(self.cfg.state_dir) / "masks"
        masks_dir.mkdir(parents=True, exist_ok=True)
        for kind in ("mask", "core", "ring"):
            M.save_labelmap(masks_dir / ("%ss.png" % kind if kind != "mask"
                                         else "labelmap.png"),
                            {a + 1: g[kind] for a, g in geo.items()})
        M.annotate(base1, {a + 1: g["mask"] for a, g in geo.items()},
                   Path(self.cfg.state_dir) / "masks_overlay.png")

    def _persist_state(self, doc, base_frame):
        text = dumps(doc, indent=1, sort_keys=True)
        active_path(self.cfg).write_text(text)
        (Path(self.cfg.state_dir)
         / ("calibration-%s.json" % doc["profile"])).write_text(text)
        np.savez_compressed(Path(self.cfg.state_dir) / "baseline.npz",
                            frame=base_frame)
        (Path(self.cfg.state_dir) / "baseline_meta.json").write_text(
            dumps({"ts": time.time(), "exposure": doc.get("exposure_time_abs")}))

    def run(self, profile=None, skip_fingerprints=False):
        cfg = self.cfg
        addrs = self.api.optical_addrs()
        if not addrs:
            raise CalibrationError("no optical lamps on the bus (HIL_OPTICAL_SHORTS)")
        self.log("lamps: %s" % addrs)
        self.api.off_many(addrs)
        lock_mode, probe = self._require_photometric(addrs)

        identities = identity_mod.collect(self.api, only=set(addrs))
        caps = self._caps_primed(addrs)

        self.log("baseline (all off) x2 for noise floor...")
        self.api.off_many(addrs)
        base1 = self._capture()
        base2 = self.backend.capture()

        geo = self._solo_masks(addrs, base1)
        if not geo:
            raise CalibrationError("no lamp produced a mask — camera aimed correctly?")

        exposure, final_clips, exposure_floor_hit = self._tune_exposure(addrs, geo)

        self.log("re-baseline + thresholds at tuned exposure...")
        base1 = self._capture()
        base2b = self.backend.capture()
        solo_frames = self._solo_capture(addrs, geo)
        on_delta = self._crosstalk_thresholds(geo, solo_frames, base1)
        del solo_frames
        floors = self._noise_floors(geo, base1, base2b)

        ladder_addr = next(iter(geo))
        ladder, monotonic = self._ladder(ladder_addr, geo, base1, floors)

        fingerprints = {} if skip_fingerprints else \
            self._fingerprints(addrs, caps, geo, base1)
        self.api.off_many(addrs)

        self.log("fiducials...")
        fiducials = MX.pick_fiducials(base1, [g["mask"] for g in geo.values()],
                                      Path(cfg.state_dir))

        if profile is None:
            profile = self._infer_profile(base1)

        lamps = self._lamp_records(geo, identities, on_delta, floors, final_clips)
        self._save_masks(geo, base1)

        doc = {
            "version": SCHEMA_VERSION,
            "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "profile": profile,
            "camera": {"name": cfg.camera_name, "backend": self.backend.name},
            "camera_mode": {"lock": lock_mode.value, "probe_ratio": probe.ratio,
                            "capability": probe.capability.value},
            "mono_metric": MONO_METRIC,
            "baseline_mean": round(float(base1.mean()), 2),
            "exposure_time_abs": exposure,
            "exposure_floor_hit": exposure_floor_hit,
            "settle_seconds": SETTLE_S,
            "color_ramp_seconds": COLOR_RAMP_S,
            "ladder_label": ladder_addr + 1,
            "ladder": {str(k): v for k, v in ladder.items()},
            "monotonic_levels": monotonic,
            "fingerprints": fingerprints,
            "fiducials": fiducials,
            "lamps": lamps,
        }
        self._persist_state(doc, base1)
        self.backend.write_controls(exposure=None)
        self.log("calibration v%d saved (profile=%s, capability=%s)"
                 % (SCHEMA_VERSION, profile, probe.capability.value))
        return doc

    def refresh_fingerprints(self):
        doc = load(self.cfg)
        label_to_short, _missing = identity_mod.refresh_short_addresses(
            self.api, doc["lamps"])
        geo = {label_to_short[label]: g
               for label, g in M.load_geometry(self.cfg).items()
               if label in label_to_short}
        addrs = self.api.optical_addrs()
        caps = {d["short_address"]: d.get("capabilities", {})
                for d in self.api.devices()["physical_devices"]}
        self.log("fingerprints-only: fresh all-off baseline...")
        self.api.off_many(addrs)
        base = self._capture()
        doc["fingerprints"] = self._fingerprints(addrs, caps, geo, base)
        self.api.off_many(addrs)
        doc["fingerprints_updated"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        self._persist_state(doc, base)
        return doc
