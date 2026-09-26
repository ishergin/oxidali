import dataclasses
import json
from dataclasses import dataclass
from typing import Optional

import numpy as np

Short = int
Label = int
WireAddress = int

FAILURE_LINE_CHARS = 1000


def failure_line(longrepr):
    message = getattr(getattr(longrepr, "reprcrash", None), "message", None)
    if not message:
        lines = [line for line in str(longrepr).splitlines() if line.strip()]
        message = lines[-1] if lines else ""
    return " ".join(message.split())[:FAILURE_LINE_CHARS]


def np_json_default(obj):
    if isinstance(obj, np.integer):
        return int(obj)
    if isinstance(obj, np.floating):
        return float(obj)
    if isinstance(obj, np.ndarray):
        return obj.tolist()
    if dataclasses.is_dataclass(obj):
        return dataclasses.asdict(obj)
    raise TypeError("not JSON serializable: %r" % type(obj))


def dumps(obj, **kw):
    return json.dumps(obj, default=np_json_default, **kw)


@dataclass
class LampIdentity:
    gtin: Optional[int]
    identification_number: Optional[int]
    short_address: int
    label: int

    @property
    def key(self):
        return (self.gtin, self.identification_number)


