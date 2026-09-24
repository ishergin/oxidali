import cv2
import numpy as np

from hil.camera.color import luminance_linear, mccamy_cct, xy_chromaticity

CLIP_LEVEL = 250.0
CHANGED_DELTA = 25.0
MIN_SIGNAL_PIXELS = 30
MIN_LAMP_DELTA = 20.0
UNLIT_FLOOR = 40.0


def _baseline_diffs(baseline_bgr, mask, core, ring, lum, mask_mc, max_channel):
    base_mc = baseline_bgr.max(axis=2)
    blum = luminance_linear(baseline_bgr[..., ::-1][mask] / 255.0)
    diff_sum_lum = float((lum - blum).sum())
    changed_fraction = float(
        (np.abs(mask_mc - base_mc[mask]) > CHANGED_DELTA).mean()) if mask_mc.size else 0.0
    core_delta = None
    if core is not None and ring is not None and ring.any():
        d = np.abs(max_channel - base_mc)
        core_delta = float(d[core].mean() - d[ring].mean())
    return diff_sum_lum, changed_fraction, core_delta


def _brightest_lit(region_mc, lit):
    sig = np.zeros(region_mc.shape, bool)
    cand = np.flatnonzero(lit)
    if cand.size:
        order = cand[np.argsort(region_mc[cand])]
        sig[order[-max(MIN_SIGNAL_PIXELS, order.size // 10):]] = True
    return sig


def _colour_sample(frame_bgr, baseline_bgr, colour_region, max_channel):
    region_mc = max_channel[colour_region]
    if not region_mc.size:
        return np.zeros(3), 0, "empty colour region"
    region_rgb = frame_bgr[..., ::-1][colour_region] / 255.0
    unclipped = region_mc < CLIP_LEVEL
    if baseline_bgr is not None:
        lamp_rise = region_mc - baseline_bgr.max(axis=2)[colour_region]
        lit = unclipped & (lamp_rise >= MIN_LAMP_DELTA)
        thr = max(MIN_LAMP_DELTA, float(np.quantile(lamp_rise, 0.80)))
        sig = unclipped & (lamp_rise >= thr)
    else:
        lit = unclipped & (region_mc > UNLIT_FLOOR)
        sig = lit
    if sig.sum() < MIN_SIGNAL_PIXELS:
        sig = _brightest_lit(region_mc, lit)
    count = int(np.count_nonzero(sig))
    if count < MIN_SIGNAL_PIXELS:
        return np.zeros(3), count, ("colour region fully clipped" if not unclipped.any()
                                    else "only %d lit unclipped pixels" % count)
    return region_rgb[sig].mean(axis=0) * 255.0, count, None


COLOUR_STAT_KEYS = ("mean_rgb", "rb_ratio", "hue", "sat", "xy", "cct_mccamy")


def _colour_stats(mean_rgb, blind=None):
    if blind is not None:
        return dict.fromkeys(COLOUR_STAT_KEYS)
    r, g, b = (mean_rgb / 255.0).tolist()
    x, y = xy_chromaticity(np.array([[[r, g, b]]]))
    x, y = float(np.nan_to_num(x)[0][0]), float(np.nan_to_num(y)[0][0])
    rb_ratio = float((mean_rgb[0] + 1e-6) / (mean_rgb[2] + 1e-6))
    hsv = cv2.cvtColor(np.uint8([[mean_rgb[::-1]]]), cv2.COLOR_BGR2HSV)[0][0]
    return {
        "mean_rgb": [round(float(v), 1) for v in mean_rgb],
        "rb_ratio": round(rb_ratio, 3),
        "hue": int(hsv[0]) * 2,
        "sat": round(int(hsv[1]) / 255.0, 3),
        "xy": [round(x, 4), round(y, 4)],
        "cct_mccamy": int(np.clip(np.nan_to_num(mccamy_cct(x, y)), 0, 20000)),
    }


def lamp_metrics(frame_bgr, mask, core=None, ring=None, baseline_bgr=None):
    rgb = frame_bgr[..., ::-1][mask] / 255.0
    lum = luminance_linear(rgb)
    sum_lum = float(lum.sum())
    max_channel = frame_bgr.max(axis=2)
    mask_mc = max_channel[mask]
    clip_fraction = float((mask_mc >= CLIP_LEVEL).mean()) if mask_mc.size else 0.0

    diff_sum_lum = changed_fraction = core_delta = None
    if baseline_bgr is not None:
        diff_sum_lum, changed_fraction, core_delta = _baseline_diffs(
            baseline_bgr, mask, core, ring, lum, mask_mc, max_channel)

    colour_region = mask | ring if ring is not None else mask
    colour_mc = max_channel[colour_region]
    colour_clip_fraction = (float((colour_mc >= CLIP_LEVEL).mean())
                            if colour_mc.size else 0.0)
    mean_rgb, signal_pixels, colour_blind = _colour_sample(
        frame_bgr, baseline_bgr, colour_region, max_channel)
    return {
        "sum_lum": round(sum_lum, 2),
        "diff_sum_lum": None if diff_sum_lum is None else round(diff_sum_lum, 2),
        "changed_fraction": None if changed_fraction is None else round(changed_fraction, 4),
        "core_delta": None if core_delta is None else round(core_delta, 1),
        "clip_fraction": round(clip_fraction, 4),
        "colour_clip_fraction": round(colour_clip_fraction, 4),
        **_colour_stats(mean_rgb, colour_blind),
        "signal_pixels": signal_pixels,
        "colour_blind": colour_blind,
    }


def ring_clip_fraction(frame_bgr, core, ring):
    mc = frame_bgr.max(axis=2)
    if not ring.any():
        return 0.0
    return float((mc[ring] >= CLIP_LEVEL).mean())


FID_SIZE = 64
MIN_TEXTURE_VAR = 25.0


def pick_fiducials(baseline_bgr, exclude_masks, state_dir, count=3):
    gray = cv2.cvtColor(baseline_bgr.astype(np.uint8), cv2.COLOR_BGR2GRAY)
    lap = np.abs(cv2.Laplacian(cv2.GaussianBlur(gray, (5, 5), 0), cv2.CV_32F))
    h, w = gray.shape
    keep_out = np.zeros((h, w), bool)
    for mask in exclude_masks:
        keep_out |= cv2.dilate(mask.astype(np.uint8),
                               np.ones((81, 81), np.uint8)).astype(bool)
    scored = []
    for yy in range(0, h - FID_SIZE, FID_SIZE):
        for xx in range(0, w - FID_SIZE, FID_SIZE):
            if keep_out[yy:yy + FID_SIZE, xx:xx + FID_SIZE].any():
                continue
            scored.append((float(lap[yy:yy + FID_SIZE, xx:xx + FID_SIZE].var()), xx, yy))
    scored.sort(reverse=True)
    fids, taken = [], []
    for var, xx, yy in scored:
        if len(fids) == count:
            break
        if var < MIN_TEXTURE_VAR:
            break
        if any(abs(xx - tx) < 300 and abs(yy - ty) < 300 for tx, ty in taken):
            continue
        name = "fid_%d.png" % len(fids)
        cv2.imwrite(str(state_dir / name), gray[yy:yy + FID_SIZE, xx:xx + FID_SIZE])
        fids.append({"bbox": [xx, yy, FID_SIZE, FID_SIZE], "template": name,
                     "texture_var": round(var, 1)})
        taken.append((xx, yy))
    return fids


def check_fiducials(frame_bgr, fiducials, state_dir, threshold=0.75):
    gray = cv2.cvtColor(frame_bgr.astype(np.uint8), cv2.COLOR_BGR2GRAY)
    worst = 1.0
    for fid in fiducials:
        x, y, w, h = fid["bbox"]
        tpl = cv2.imread(str(state_dir / fid["template"]), cv2.IMREAD_GRAYSCALE)
        if tpl is None:
            continue
        margin = 24
        region = gray[max(0, y - margin):y + h + margin,
                      max(0, x - margin):x + w + margin]
        if region.shape[0] < tpl.shape[0] or region.shape[1] < tpl.shape[1]:
            continue
        score = float(cv2.matchTemplate(region, tpl, cv2.TM_CCOEFF_NORMED).max())
        worst = min(worst, score)
    return worst >= threshold, round(worst, 3)
