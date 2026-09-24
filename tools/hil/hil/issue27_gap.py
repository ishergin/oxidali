import json
import sys
import time

from hil import arbiter as arb
from hil.issue27_ab import ATTRIB_WINDOW_S, Reader


def deficit(frame):
    if not frame.durations or frame.first_level != 0:
        return None
    lows = [d for lvl, d in arb.pulses(frame)
            if lvl == 0 and d < arb.SINGLE_DOUBLE_SPLIT_US]
    if len(lows) < 3:
        return None
    return frame.durations[0] - arb._mean(lows[1:])


def run(port, out_path, rounds=14, gaps_ms=(10, 30, 100, 300, 1000, 3000),
        frame=0x0790):
    from hil.api import Client
    from hil.config import HilConfig

    api = Client(HilConfig())
    rd = Reader(port, out_path)
    rd.start()
    rd.note("issue27_gap start frame=0x%04X rounds=%d" % (frame, rounds))
    time.sleep(1.0)

    rows = []
    for r in range(rounds):
        for gap_ms in gaps_ms:
            time.sleep(3.0)
            t0 = time.time()
            try:
                api.raw(frame, expects_backward=True)
            except Exception as exc:
                rd.note("gap round=%d gap_ms=%d first_err=%s" % (r, gap_ms, exc))
                continue
            t1 = time.time()
            time.sleep(gap_ms / 1000.0)
            t2 = time.time()
            try:
                api.raw(frame, expects_backward=True)
            except Exception as exc:
                rd.note("gap round=%d gap_ms=%d second_err=%s" % (r, gap_ms, exc))
                continue
            t3 = time.time()
            rd.note("pair round=%d gap_ms=%d cold=[%.6f,%.6f] warm=[%.6f,%.6f]"
                    % (r, gap_ms, t0, t1, t2, t3))
            time.sleep(0.3)

            cold = _one_forward(rd, t0, t1 + ATTRIB_WINDOW_S)
            warm = _one_forward(rd, t2, t3 + ATTRIB_WINDOW_S)
            dc = deficit(cold) if cold else None
            dw = deficit(warm) if warm else None
            rows.append({"round": r, "gap_ms": gap_ms, "cold": dc, "warm": dw})
            print("round=%2d gap=%5dms  cold_deficit=%s  warm_deficit=%s" % (
                r, gap_ms,
                "%7.1f" % dc if dc is not None else "   n/a ",
                "%7.1f" % dw if dw is not None else "   n/a "), flush=True)

    rd.note("issue27_gap end")
    time.sleep(0.5)
    rd.close()
    with open(out_path + ".json", "w") as fh:
        json.dump(rows, fh, indent=1)
    _report(rows)
    return rows


def _one_forward(rd, t0, t1):
    got = [f for f in rd.frames_between(t0, t1)]
    fwd = []
    for f in got:
        b, _ = arb.decode_bytes(f)
        if b is not None and len(b) == 2:
            fwd.append(f)
    return fwd[0] if len(fwd) == 1 else None


def _report(rows):
    gaps = sorted({r["gap_ms"] for r in rows})
    print("\n%-10s %6s %26s %26s" % ("gap", "pairs", "1st frame deficit (us)",
                                     "2nd frame deficit (us)"))
    print("-" * 74)
    for g in gaps:
        rs = [r for r in rows if r["gap_ms"] == g]
        c = [r["cold"] for r in rs if r["cold"] is not None]
        w = [r["warm"] for r in rs if r["warm"] is not None]
        print("%-10s %6d %26s %26s" % (
            "%d ms" % g, len(rs),
            "mean=%7.1f n=%d" % (arb._mean(c), len(c)) if c else "n/a",
            "mean=%7.1f n=%d" % (arb._mean(w), len(w)) if w else "n/a"))


def load_report(path):
    with open(path) as fh:
        _report(json.load(fh))


def main(argv):
    if len(argv) > 1 and argv[1] == "report":
        load_report(argv[2])
        return 0
    port = argv[1] if len(argv) > 1 else "/dev/cu.usbmodem11401"
    out = argv[2] if len(argv) > 2 else "runs/arbiter/gap.txt"
    rounds = int(argv[3]) if len(argv) > 3 else 14
    run(port, out, rounds=rounds)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
