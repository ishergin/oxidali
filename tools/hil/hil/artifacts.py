import re
from pathlib import Path

import cv2
import numpy as np

from hil.results import dumps


def _sanitize(node_id: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", node_id).strip("_")[:150]


class Artifacts:
    def __init__(self, run_dir: Path, node_id: str):
        self.dir = Path(run_dir) / _sanitize(node_id)
        self.dir.mkdir(parents=True, exist_ok=True)
        self.attached = []

    def _path(self, name: str) -> Path:
        p = self.dir / name
        self.attached.append(str(p))
        return p

    def attach_png(self, name: str, frame_bgr) -> str:
        if not name.endswith(".png"):
            name += ".png"
        p = self._path(name)
        cv2.imwrite(str(p), np.clip(frame_bgr, 0, 255).astype("uint8"))
        return str(p)

    def attach_json(self, name: str, obj) -> str:
        if not name.endswith(".json"):
            name += ".json"
        p = self._path(name)
        p.write_text(dumps(obj, indent=1, sort_keys=True))
        return str(p)

    def attach_text(self, name: str, text: str) -> str:
        p = self._path(name)
        p.write_text(text)
        return str(p)
