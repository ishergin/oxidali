import dataclasses
import json
from dataclasses import dataclass
from typing import Optional

import numpy as np

Short = int
Label = int
WireAddress = int


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


