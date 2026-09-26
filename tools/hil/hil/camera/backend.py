import enum
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass

import numpy as np

from hil.camera.capture import band_linear_mean, CameraError as _CameraError

WARMUP_FRAMES = 20
AVG_FRAMES = 4

LINEAR_RATIO = 3.0


CameraError = _CameraError


class LockMode(enum.Enum):
    UVC_LOCKED = "uvc-locked"
    UVC_MISMATCH = "uvc-mismatch"
    UNLOCKED = "unlocked"


class Capability(enum.Enum):
    ORDERING_ONLY = "ordering-only"
    PHOTOMETRIC = "photometric"
    UNKNOWN = "unknown"


FLAT_RATIO = 1.25

REENUMERATE = ("run `vendor/uvc-util/uvc-util -d`, then `hil camera-server --restart`: "
               "AVFoundation hands the camera over only once the device is enumerated "
               "again, and `--spawn-terminal` leaves a live server as it is")
SPAWN = "start it with `hil camera-server --spawn-terminal`"


@dataclass
class ProbeResult:
    ratio: float
    capability: Capability
    band_px: int = 0


class CameraBackend(ABC):
    name = "abstract"

    @abstractmethod
    def capture(self, warmup=WARMUP_FRAMES, avg=AVG_FRAMES) -> np.ndarray:
        pass

    @abstractmethod
    def ensure_locked(self) -> LockMode:
        pass

    @abstractmethod
    def write_controls(self, exposure=None, wb=None) -> None:
        pass

    def close(self) -> None:
        pass

    def effect_probe(self, light=None) -> ProbeResult:
        if light:
            light(True)
            time.sleep(1.6)
        try:
            frames = {}
            for exp in (200, 20):
                self.write_controls(exposure=exp)
                time.sleep(0.8)
                frames[exp] = self.capture(warmup=8, avg=3)
            self.write_controls(exposure=None)
        finally:
            if light:
                light(False)
        hi_lin, band = band_linear_mean(frames[200])
        lo_lin, _ = band_linear_mean(frames[20], band)
        ratio = hi_lin / max(lo_lin, 1e-9)
        return ProbeResult(
            ratio=round(ratio, 2),
            capability=Capability.PHOTOMETRIC if ratio > LINEAR_RATIO
            else Capability.ORDERING_ONLY,
            band_px=int(band.sum()))


def probe_and_select(cfg):
    from hil.camera.backends import DirectBackend, FrameServerBackend
    try:
        direct = DirectBackend(cfg)
        direct.capture(warmup=3, avg=1)
        return direct
    except CameraError:
        pass
    server = FrameServerBackend(cfg)
    if not server.alive():
        raise CameraError("no camera path: direct capture is TCC-denied and %s"
                          % server.down_advice())
    return server
