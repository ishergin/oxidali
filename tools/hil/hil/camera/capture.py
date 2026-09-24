import time

import cv2
import numpy as np

from hil.camera.color import luminance_linear


class CameraError(RuntimeError):
    pass

CAPTURE_W, CAPTURE_H = 1920, 1080
LIVE_GAP_S = 0.015
LIVE_STREAK = 3
MAX_GRABS = 60
BAND_LO, BAND_HI = 80.0, 240.0
BAND_MIN_PX = 1000


def open_session(index, w=CAPTURE_W, h=CAPTURE_H):
    cap = cv2.VideoCapture(index, cv2.CAP_AVFOUNDATION)
    if not cap.isOpened():
        cap.release()
        raise CameraError(
            "AVFoundation device %d did not open — the camera is unplugged, "
            "held by another process, or this process has no camera TCC grant"
            % index)
    cap.set(cv2.CAP_PROP_FRAME_WIDTH, w)
    cap.set(cv2.CAP_PROP_FRAME_HEIGHT, h)
    got_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH) or 0)
    got_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT) or 0)
    if (got_w, got_h) != (w, h):
        cap.release()
        raise CameraError(
            "AVFoundation device %d streams %dx%d, not the %dx%d every "
            "calibration threshold was measured at — the camera has "
            "re-enumerated in a degraded mode. Replug it (different port, no "
            "hub) and check the mode list with: ffmpeg -f avfoundation "
            "-video_size %dx%d -i %d -frames:v 1 -y /tmp/probe.png"
            % (index, got_w, got_h, w, h, w, h, index))
    return cap


def drain_to_live(cap, max_grabs=MAX_GRABS):
    live_streak = 0
    for _ in range(max_grabs):
        t0 = time.time()
        cap.grab()
        live_streak = live_streak + 1 if time.time() - t0 > LIVE_GAP_S else 0
        if live_streak >= LIVE_STREAK:
            break


def avg_frames(cap, n):
    acc, k = None, 0
    for _ in range(int(n)):
        ok, frame = cap.read()
        if not ok or frame is None:
            continue
        f = frame.astype(np.float32)
        acc = f if acc is None else acc + f
        k += 1
    return None if k == 0 else acc / k


def band_linear_mean(frame_bgr, mask=None):
    mc = frame_bgr.max(axis=2)
    if mask is None:
        mask = (mc > BAND_LO) & (mc < BAND_HI)
        if mask.sum() < BAND_MIN_PX:
            mask = mc > -1
    return float(luminance_linear(frame_bgr[..., ::-1] / 255.0)[mask].mean()), mask
