import json
import os
from pathlib import Path


def write_json(path, data):
    path = Path(path)
    tmp = path.with_name(path.name + ".tmp")
    with open(tmp, "w", encoding="utf-8") as out:
        out.write(json.dumps(data, indent=1, sort_keys=True, ensure_ascii=False))
        out.flush()
        os.fsync(out.fileno())
    os.replace(tmp, path)
    folder = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(folder)
    finally:
        os.close(folder)
