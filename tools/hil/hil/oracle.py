import collections
import time

from hil.camera import metrics as MX
from hil.camera.calibrate import COLOR_RAMP_S, SETTLE_S, colour_ok

BASELINE_TTL_S = 240.0


class OpticalAssertError(AssertionError):
    pass


class ColourBlindError(OpticalAssertError):
    pass


class CameraOracle:
    def __init__(self, backend, calibration, geometry, cfg, artifacts=None):
        self.backend = backend
        self.cal = calibration
        self.geometry = geometry
        self.cfg = cfg
        self.artifacts = artifacts
        self.thresholds = {l["label"]: l["thresholds"] for l in calibration["lamps"]}
        self._baseline = None
        self.retry_causes = collections.Counter()
        self.retries = 0

    def adopt_calibration(self, calibration):
        self.cal = calibration
        self.thresholds = {l["label"]: l["thresholds"] for l in calibration["lamps"]}
        self._baseline = None

    def fresh_baseline(self, api):
        if self._baseline and time.monotonic() - self._baseline[1] < BASELINE_TTL_S:
            return self._baseline[0]
        api.off_many(api.optical_addrs())
        time.sleep(SETTLE_S)
        frame = self.backend.capture()
        self._check_lighting_regime(frame)
        self._check_fiducials(frame)
        self._baseline = (frame, time.monotonic())
        return frame

    def _check_lighting_regime(self, frame):
        expected = self.cal.get("baseline_mean")
        if not expected:
            return
        actual = float(frame.mean())
        if actual > 3.0 * expected or actual < expected / 3.0:
            raise OpticalAssertError(
                "lighting regime changed (baseline mean %.1f vs calibrated %.1f, "
                "profile %r) — recalibrate: hil calibrate"
                % (actual, expected, self.cal.get("profile")))

    def invalidate_baseline(self):
        self._baseline = None

    def _check_fiducials(self, frame):
        ok, score = MX.check_fiducials(frame, self.cal.get("fiducials", []),
                                       self.cfg.state_dir)
        if not ok:
            raise OpticalAssertError(
                "camera moved (fiducial score %.3f) — recalibrate: hil calibrate" % score)

    def measure(self, label, name=None):
        if self._baseline is None:
            raise OpticalAssertError("no baseline — call fresh_baseline(api) first")
        frame = self.backend.capture()
        g = self.geometry[label]
        m = MX.lamp_metrics(frame, g["mask"], g["core"], g["ring"],
                            baseline_bgr=self._baseline[0])
        if self.artifacts and name:
            self.artifacts.attach_png(name, frame)
            self.artifacts.attach_json(name + "_metrics", m)
        return m

    def _count_retry(self, cause):
        self.retry_causes[cause] += 1
        self.retries += 1

    def _assert(self, check, label, name, window=None):
        time.sleep(SETTLE_S)
        try:
            return check(self.measure(label, name))
        except ColourBlindError:
            raise
        except AssertionError:
            if window is not None and window.contaminated:
                self._count_retry("contaminated_window")
                if self.artifacts:
                    self.artifacts.attach_json(
                        (name or "assert") + "_retry",
                        {"reason": "foreign frames in sniffer window",
                         "foreign": [f["decoded"] for f in window.foreign_frames()][:8]})
                time.sleep(1.0)
                return check(self.measure(label, (name or "m") + "_retry"))
            raise

    def assert_on(self, label, window=None, name="on", resend=None):
        thr = self.thresholds[label]["on_delta"]

        def check(m):
            focal_on = m["core_delta"] is not None and m["core_delta"] > thr
            clipped_on = (m["clip_fraction"] > 0.75
                          and (m["changed_fraction"] or 0.0) > 0.5)
            if not (focal_on or clipped_on):
                raise OpticalAssertError(
                    "lamp %s expected ON: core_delta=%s <= threshold %s "
                    "(clip=%.2f changed=%s)"
                    % (label, m["core_delta"], thr, m["clip_fraction"],
                       m["changed_fraction"]))
            return m
        return self._assert_with_resend(check, label, name, window, resend)

    def assert_off(self, label, window=None, name="off", resend=None):
        thr = self.thresholds[label]["on_delta"]

        def check(m):
            if not (m["core_delta"] is not None and m["core_delta"] <= thr):
                raise OpticalAssertError(
                    "lamp %s expected OFF: core_delta=%s > threshold %s"
                    % (label, m["core_delta"], thr))
            return m
        return self._assert_with_resend(check, label, name, window, resend)

    def _assert_with_resend(self, check, label, name, window, resend):
        try:
            return self._assert(check, label, name, window)
        except OpticalAssertError:
            if resend is None:
                raise
            resend()
            self._count_retry("lost_command")
            return self._assert(check, label, name + "_resend", window)

    def require_colour(self, m, label, what="colour"):
        if m["colour_blind"] is not None:
            raise ColourBlindError(
                "lamp %s: %s unmeasurable — %s (signal_pixels=%s, %.0f%% of the "
                "colour region clipped); measure at a level that does not clip, "
                "or recalibrate the exposure: hil calibrate"
                % (label, what, m["colour_blind"], m["signal_pixels"],
                   100.0 * m["colour_clip_fraction"]))
        return m

    def assert_hue(self, label, color_name, window=None, resend=None):
        time.sleep(COLOR_RAMP_S)

        def check(m):
            self.require_colour(m, label, "hue")
            if not colour_ok(color_name, m):
                raise OpticalAssertError(
                    "lamp %s expected %s: hue=%s sat=%.2f rgb=%s"
                    % (label, color_name, m["hue"], m["sat"], m["mean_rgb"]))
            return m
        try:
            return self._assert(check, label, "hue_%s" % color_name, window)
        except ColourBlindError:
            raise
        except OpticalAssertError:
            if resend is not None:
                resend()
            time.sleep(COLOR_RAMP_S)
            self._count_retry("gear_colour_lag")
            return self._assert(check, label, "hue_%s_lagretry" % color_name, window)

    def judge_cct_order(self, ratios, min_spread=1.15):
        warm, cool = ratios[2700], ratios[6500]
        ordered = warm >= cool * min_spread
        if self.artifacts:
            self.artifacts.attach_json("cct_verdict",
                                       {"ratios": ratios, "ordered": ordered})
        if not ordered:
            raise OpticalAssertError(
                "warm/cool rb ordering broken: 2700K=%s 6500K=%s" % (warm, cool))

    def mono_metric(self, measurement):
        return measurement["diff_sum_lum"]

    def assert_strict_order(self, values, context=""):
        pairs = list(zip(values, values[1:]))
        if not all(a < b for a, b in pairs):
            raise OpticalAssertError("expected strictly increasing %s: %s"
                                     % (context, values))

    def cct_fingerprint_spread(self):
        for key, fp in self.cal.get("fingerprints", {}).items():
            if key.startswith("cct_addr_") and "2700" in fp and "6500" in fp:
                addr = int(key.rsplit("_", 1)[-1])
                label = next((l["label"] for l in self.cal["lamps"]
                              if l["short_address"] == addr), None)
                warm, cool = fp["2700"]["rb_ratio"], fp["6500"]["rb_ratio"]
                return label, warm / max(cool, 1e-9)
        return None, 0.0
