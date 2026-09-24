import json
import statistics
import time
from pathlib import Path

from hil.camera.backends import resolve_index
from hil.camera.capture import (avg_frames, band_linear_mean, drain_to_live,
                                open_session)
from hil.camera.controls import UvcUtilControl
from hil.config import load as load_config

EXPO_HI, EXPO_LO = 2000, 200


def _open(cfg):
    return open_session(resolve_index(cfg))


def _avg(cap, n=3):
    f = avg_frames(cap, n)
    if f is None:
        raise RuntimeError("no frames")
    return f


def run():
    import io
    import sys
    import traceback
    cfg = load_config()
    log = Path(cfg.state_dir) / "camera_bench.log"
    buf = io.StringIO()

    class Tee:
        def write(self, s):
            sys.__stdout__.write(s)
            buf.write(s)

        def flush(self):
            sys.__stdout__.flush()
    old = sys.stdout
    sys.stdout = Tee()
    try:
        return _run(cfg)
    except Exception:
        traceback.print_exc(file=sys.stdout)
        return 1
    finally:
        sys.stdout = old
        log.write_text(buf.getvalue())


def _phase_honesty(cfg, ctl):
    print("== 1. exposure honesty (linear ratio, %dx command) ==" % (EXPO_HI // EXPO_LO))
    cap = _open(cfg)
    for _ in range(20):
        cap.read()
    ctl.set("exposure-time-abs", EXPO_HI)
    time.sleep(1.0)
    drain_to_live(cap)
    hi = _avg(cap)
    ctl.set("exposure-time-abs", EXPO_LO)
    time.sleep(1.0)
    drain_to_live(cap)
    lo = _avg(cap)
    hi_lin, band = band_linear_mean(hi)
    lo_lin, _ = band_linear_mean(lo, band)
    frag = {"exposure_ratio_linear": round(hi_lin / max(lo_lin, 1e-9), 2),
            "mean_hi": round(float(hi.mean()), 1),
            "mean_lo": round(float(lo.mean()), 1)}
    print("   ratio=%s (hi mean %.1f, lo mean %.1f)"
          % (frag["exposure_ratio_linear"], hi.mean(), lo.mean()))
    return frag, cap, float(hi.mean()), float(lo.mean())


def _phase_reopen_persistence(cfg, ctl, cap, hi_mean, in_stream_lo):
    print("== 2. control persistence across close/reopen ==")
    cap.release()
    ctl.set("exposure-time-abs", EXPO_HI)
    time.sleep(0.5)
    cap = _open(cfg)
    for _ in range(10):
        cap.read()
    reopened = _avg(cap)
    readback = ctl.get("exposure-time-abs")
    survives = abs(float(reopened.mean()) - hi_mean) \
        < abs(float(reopened.mean()) - in_stream_lo)
    frag = {"controls_survive_reopen": bool(survives),
            "reopen_readback": readback,
            "reopen_mean": round(float(reopened.mean()), 1)}
    print("   set %d while closed -> reopen mean %.1f (hi was %.1f, lo %.1f), "
          "readback=%s -> survives=%s"
          % (EXPO_HI, reopened.mean(), hi_mean, in_stream_lo, readback, survives))
    ctl.set("exposure-time-abs", EXPO_LO)
    time.sleep(0.5)
    return frag, cap


def _phase_stream_latency(cap):
    print("== 3a. streaming latency (persistent session, drain+avg3) x10 ==")
    for _ in range(10):
        cap.read()
    stream_t, stream_means = [], []
    for _ in range(10):
        t0 = time.time()
        drain_to_live(cap)
        f = _avg(cap)
        stream_t.append(time.time() - t0)
        stream_means.append(float(f.mean()))
    cap.release()
    frag = {"stream_capture_s": {"mean": round(statistics.mean(stream_t), 3),
                                 "min": round(min(stream_t), 3),
                                 "max": round(max(stream_t), 3)},
            "stream_mean_std": round(statistics.pstdev(stream_means), 3)}
    print("   %s (frame-mean std %.3f)"
          % (frag["stream_capture_s"], frag["stream_mean_std"]))
    return frag, stream_means


def _phase_photo_latency(cfg, stream_means):
    print("== 3b. photo latency (open+settle+avg3+close) x10 ==")
    photo_t, photo_means = [], []
    for _ in range(10):
        t0 = time.time()
        cap = _open(cfg)
        for _ in range(3):
            cap.read()
        f = _avg(cap)
        cap.release()
        photo_t.append(time.time() - t0)
        photo_means.append(float(f.mean()))
    frag = {"photo_capture_s": {"mean": round(statistics.mean(photo_t), 3),
                                "min": round(min(photo_t), 3),
                                "max": round(max(photo_t), 3)},
            "photo_mean_std": round(statistics.pstdev(photo_means), 3),
            "photo_vs_stream_brightness_delta": round(
                abs(statistics.mean(photo_means) - statistics.mean(stream_means)), 2)}
    print("   %s (frame-mean std %.3f)"
          % (frag["photo_capture_s"], frag["photo_mean_std"]))
    return frag


def _run(cfg):
    ctl = UvcUtilControl(cfg)
    out = {"camera": cfg.camera_name}

    print("== lock (manual exposure, fixed WB/gain) ==")
    out["lock_mode"] = ctl.apply_lock(exposure=EXPO_LO)

    frag, cap, hi_mean, in_stream_lo = _phase_honesty(cfg, ctl)
    out.update(frag)
    frag, cap = _phase_reopen_persistence(cfg, ctl, cap, hi_mean, in_stream_lo)
    out.update(frag)
    frag, stream_means = _phase_stream_latency(cap)
    out.update(frag)
    out.update(_phase_photo_latency(cfg, stream_means))

    path = Path(cfg.state_dir) / "camera_bench.json"
    path.write_text(json.dumps(out, indent=1, sort_keys=True))
    print("written %s" % path)
    return 0
