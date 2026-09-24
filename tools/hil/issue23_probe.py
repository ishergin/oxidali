import json
import subprocess
import sys
import time

sys.path.insert(0, "/Users/ishergin/work/dali2rust/tools/hil")
from hil.api import Api
from hil.config import Config

LOG = "/Users/ishergin/work/dali2rust/tools/hil/state/persist/serial.log"


def counters(api):
    d = api.diagnostics()["dali_worker"]
    return {k: d[k] for k in (
        "memory_bank_short_reads",
        "read_attributes_transport_aborts",
        "read_attributes_contended_aborts",
        "read_attributes_device_absent",
    )}


def main():
    api = Api(Config())
    before_lines = sum(1 for _ in open(LOG, errors="replace"))
    before = counters(api)
    print("counters before:", before)

    results = {}
    for short in (0, 1, 2, 3):
        view = api.attr_read_checked(short, groups="common_102", banks="all")
        op = view.get("op", view)
        banks = api.memory_banks(short).get("memory_banks", [])
        results[short] = {
            "status": op.get("status"),
            "error": (op.get("error") or {}).get("code"),
            "outcomes": (op.get("attribute_read_outcomes") or {}).get("memory_banks"),
            "banks": [(b["bank"], b["total_bytes_read"]) for b in banks],
        }
        print("SA%02d" % short, json.dumps(results[short]))
        time.sleep(1)

    after = counters(api)
    print("counters after:", after)
    print("delta:", {k: after[k] - before[k] for k in after})

    tail = subprocess.run(
        ["sed", "-n", "%d,$p" % (before_lines + 1), LOG],
        capture_output=True, text=True).stdout
    marks = [l for l in tail.splitlines()
             if any(t in l for t in ("01c5", "03c5", "05c5", "07c5",
                                     "c300", "c301", "a3", "9c", "98"))]
    print("wire lines captured: %d" % len(marks))
    with open("/tmp/issue23_wire.txt", "w") as f:
        f.write("\n".join(marks))


if __name__ == "__main__":
    main()
