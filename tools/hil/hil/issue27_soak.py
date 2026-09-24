import json
import random
import sys
import time

DTR0_ADDR = 0xA3
DTR1_ADDR = 0xC3
QUERY_CONTENT_DTR0 = 0x98
QUERY_CONTENT_DTR1 = 0x9C


def _query(api, short, opcode):
    frame = (((short << 1) | 1) << 8) | opcode
    try:
        r = api.raw(frame, expects_backward=True)
    except Exception:
        return None
    if not r.get("success"):
        return None
    return r.get("backward_frame")


def run(rounds=120, shorts=(0, 1, 2, 3), seed=20260801):
    from hil.api import Client
    from hil.config import HilConfig

    api = Client(HilConfig())
    rng = random.Random(seed)

    stats = {s: {"dtr0_missed": 0, "dtr1_missed": 0, "silent": 0, "other": 0,
                 "seen": 0} for s in shorts}
    prev = {"dtr0": None, "dtr1": None}
    buswide = 0
    rows = []

    for i in range(rounds):
        v0 = rng.randrange(256)
        v1 = rng.randrange(256)
        while v0 == prev["dtr0"]:
            v0 = rng.randrange(256)
        while v1 == prev["dtr1"]:
            v1 = rng.randrange(256)

        api.raw((DTR0_ADDR << 8) | v0, expects_backward=False)
        api.raw((DTR1_ADDR << 8) | v1, expects_backward=False)

        got0 = {s: _query(api, s, QUERY_CONTENT_DTR0) for s in shorts}
        got1 = {s: _query(api, s, QUERY_CONTENT_DTR1) for s in shorts}

        heard0 = heard1 = 0
        for s in shorts:
            for reg, got, want, was in (("dtr0", got0[s], v0, prev["dtr0"]),
                                        ("dtr1", got1[s], v1, prev["dtr1"])):
                if got is None:
                    stats[s]["silent"] += 1
                elif got == want:
                    stats[s]["seen"] += 1
                    if reg == "dtr0":
                        heard0 += 1
                    else:
                        heard1 += 1
                elif was is not None and got == was:
                    stats[s]["%s_missed" % reg] += 1
                else:
                    stats[s]["other"] += 1

        if heard0 == 0 or heard1 == 0:
            buswide += 1

        rows.append({"i": i, "v0": v0, "v1": v1,
                     "got0": got0, "got1": got1})
        prev["dtr0"], prev["dtr1"] = v0, v1
        if (i + 1) % 10 == 0:
            print("round %3d/%d" % (i + 1, rounds), flush=True)

    return stats, buswide, rows


def report(stats, buswide, rounds):
    print("\n%-6s %8s %8s %8s %8s %8s" %
          ("fixture", "frames", "missed", "loss%", "silent", "other"))
    print("-" * 52)
    total = rounds * 2
    for s in sorted(stats):
        st = stats[s]
        missed = st["dtr0_missed"] + st["dtr1_missed"]
        print("SA%02d   %8d %8d %7.1f%% %8d %8d" %
              (s, total, missed, 100.0 * missed / total if total else 0,
               st["silent"], st["other"]))
    print("\nbus-wide losses (nobody heard the frame): %d" % buswide)


def main(argv):
    rounds = int(argv[1]) if len(argv) > 1 else 120
    out = argv[2] if len(argv) > 2 else "runs/arbiter/soak.json"
    stats, buswide, rows = run(rounds=rounds)
    report(stats, buswide, rounds)
    with open(out, "w") as fh:
        json.dump({"stats": {str(k): v for k, v in stats.items()},
                   "buswide": buswide, "rounds": rounds,
                   "t": time.time(), "rows": rows}, fh, indent=1)
    print("-> %s" % out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
