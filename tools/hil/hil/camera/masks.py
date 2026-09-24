from pathlib import Path

import cv2
import numpy as np

FRAME_AREA_FRAC_MAX = 0.3
THRESH_ESCALATION = (0.25, 0.45, 0.65, 0.8)


def gray_absdiff(frame, baseline):
    return np.abs(frame.max(axis=2) - baseline.max(axis=2))


def detect_mask(diff_gray):
    blur = cv2.GaussianBlur(diff_gray, (15, 15), 0)
    frame_area = blur.shape[0] * blur.shape[1]
    peak = np.unravel_index(int(np.argmax(blur)), blur.shape)
    best_labels, best = None, None
    for frac in THRESH_ESCALATION:
        thresh = max(12.0, float(blur.max()) * frac)
        m = (blur > thresh).astype(np.uint8)
        m = cv2.morphologyEx(m, cv2.MORPH_OPEN, np.ones((3, 3), np.uint8))
        m = cv2.morphologyEx(m, cv2.MORPH_CLOSE, np.ones((25, 25), np.uint8))
        n, labels, stats, _ = cv2.connectedComponentsWithStats(m)
        if n < 2:
            return None
        peak_label = int(labels[peak])
        best = peak_label if peak_label > 0 else \
            1 + int(np.argmax(stats[1:, cv2.CC_STAT_AREA]))
        best_labels = labels
        if stats[best][cv2.CC_STAT_AREA] <= FRAME_AREA_FRAC_MAX * frame_area:
            break
    return best_labels == best


def _kernel(size):
    size = max(3, int(size) | 1)
    return np.ones((size, size), np.uint8)


def core_from(mask, diff_gray, quantile=0.70):
    area = int(mask.sum())
    k = int(np.clip(np.sqrt(area) / 8, 3, 31))
    eroded = cv2.erode(mask.astype(np.uint8), _kernel(k)).astype(bool)
    thr = float(np.quantile(diff_gray[mask], quantile)) if area else 0.0
    core = eroded & (diff_gray >= thr)
    return core if core.sum() >= 30 else (mask & (diff_gray >= thr))


def ring_from(mask, inner=25, outer=61):
    m = mask.astype(np.uint8)
    return (cv2.dilate(m, _kernel(outer)).astype(bool)
            & ~cv2.dilate(m, _kernel(inner)).astype(bool))


def save_labelmap(path, masks_by_label):
    if 0 in masks_by_label:
        raise ValueError("label 0 is reserved for background (use short_address+1)")
    shape = next(iter(masks_by_label.values())).shape
    out = np.zeros(shape, np.uint8)
    for label in sorted(masks_by_label):
        out[masks_by_label[label]] = label
    cv2.imwrite(str(path), out)
    return out


def load_labelmap(path):
    img = cv2.imread(str(path), cv2.IMREAD_GRAYSCALE)
    if img is None:
        raise FileNotFoundError(path)
    return {int(l): (img == l) for l in np.unique(img) if l != 0}


def load_masks(cfg):
    d = Path(cfg.state_dir) / "masks"
    return {kind: load_labelmap(d / ("%s.png" % kind))
            for kind in ("labelmap", "cores", "rings")}


def load_geometry(cfg):
    maps = load_masks(cfg)
    return {label: {"mask": maps["labelmap"][label],
                    "core": maps["cores"].get(label, maps["labelmap"][label]),
                    "ring": maps["rings"].get(label, maps["labelmap"][label])}
            for label in maps["labelmap"]}


def annotate(frame, masks_by_label, path):
    img = np.clip(frame, 0, 255).astype(np.uint8).copy()
    for label, mask in masks_by_label.items():
        contours, _ = cv2.findContours(mask.astype(np.uint8),
                                       cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        cv2.drawContours(img, contours, -1, (0, 255, 0), 2)
        ys, xs = np.where(mask)
        if xs.size:
            cv2.putText(img, str(label), (int(xs.mean()), max(18, int(ys.mean()))),
                        cv2.FONT_HERSHEY_SIMPLEX, 1.0, (0, 255, 0), 2)
    cv2.imwrite(str(path), img)
