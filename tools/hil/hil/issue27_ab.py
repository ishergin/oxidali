import json
import random
import sys
import threading
import time

from hil import arbiter as arb

DEFAULT_FRAME = 0x0790
ATTRIB_WINDOW_S = 0.6


class Reader(threading.Thread):
    daemon = True

    def __init__(self, port, out_path, baud=115200):
        super().__init__()
        import serial

        self.ser = serial.Serial(port, baud, timeout=0.01)
        self.fh = open(out_path, "w")
        self.lines = []
        self.lock = threading.Lock()
        self.stop_flag = False

    def run(self):
        buf = b""
        while not self.stop_flag:
            chunk = self.ser.read(4096)
            now = time.time()
            if not chunk:
                continue
            buf += chunk
            while b"\n" in buf:
                raw, buf = buf.split(b"\n", 1)
                line = raw.decode("utf-8", "replace").rstrip("\r")
                if not line:
                    continue
                with self.lock:
                    self.lines.append((now, line))
                    self.fh.write("%.6f %s\n" % (now, line))
                    self.fh.flush()

    def note(self, text):
        with self.lock:
            self.fh.write("# %.6f %s\n" % (time.time(), text))
            self.fh.flush()

    def frames_between(self, t0, t1):
        with self.lock:
            rows = [(t, ln) for (t, ln) in self.lines if t0 <= t <= t1]
        out = []
        for t, ln in rows:
            f = arb.parse_line(ln, t)
            if f:
                out.append(f)
        return out

    def close(self):
        self.stop_flag = True
        time.sleep(0.1)
        try:
            self.ser.close()
        finally:
            self.fh.close()


def run(port, out_path, trials=60, frame=DEFAULT_FRAME, idle_choices=(0.05, 0.3, 1.0, 3.0)):
    from hil.api import Client
    from hil.config import HilConfig

    api = Client(HilConfig())
    rd = Reader(port, out_path)
    rd.start()
    rd.note("issue27_ab start frame=0x%04X trials=%d" % (frame, trials))
    time.sleep(1.0)

    want_hex = "%04x" % frame
    results = []
    rng = random.Random(20260801)

    for i in range(trials):
        idle = rng.choice(idle_choices)
        time.sleep(idle)
        t0 = time.time()
        try:
            resp = api.raw(frame, expects_backward=True)
        except Exception as exc:
            resp = {"error": "http:%s" % exc}
        t1 = time.time()
        rd.note("ours trial=%d idle=%.3f t0=%.6f t1=%.6f resp=%s"
                % (i, idle, t0, t1, json.dumps(resp, sort_keys=True)))
        time.sleep(0.25)

        got = rd.frames_between(t0, t1 + ATTRIB_WINDOW_S)
        fwd = []
        for f in got:
            b, _ = arb.decode_bytes(f)
            if b is not None and b.hex() == want_hex:
                fwd.append(f)
        results.append({
            "i": i, "idle": idle, "t0": t0, "t1": t1, "resp": resp,
            "n_match": len(fwd), "frames": fwd, "all": got,
        })
        print("trial %3d idle=%.2fs matched=%d dut_success=%s" %
              (i, idle, len(fwd), resp.get("success")), flush=True)

    rd.note("issue27_ab end")
    time.sleep(0.5)
    rd.close()
    return results


def _pct(xs, p):
    if not xs:
        return float("nan")
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(p * (len(s) - 1))))]


def _first_pulse_table(ours, ref, backward):
    print("\n%-24s %6s | %-28s | %-28s | %s" %
          ("arm", "frames", "first low half-bit (us)", "other low half-bits (us)", "deficit"))
    print("-" * 116)
    for label, fs in (("OUR TX (dali2rust P4)", ours),
                      ("REFERENCE TX (WB)", ref),
                      ("GEAR backward frames", backward)):
        firsts, rest = [], []
        for f in fs:
            lows = [d for i, d in enumerate(arb.pulses(f))
                    if d[0] == 0 and d[1] < arb.SINGLE_DOUBLE_SPLIT_US]
            lows = [d for (_lvl, d) in
                    [(lvl, d) for lvl, d in arb.pulses(f)
                     if lvl == 0 and d < arb.SINGLE_DOUBLE_SPLIT_US]]
            if not f.durations:
                continue
            if f.first_level == 0:
                firsts.append(f.durations[0])
                rest.extend(lows[1:])
            else:
                rest.extend(lows)
        if not fs:
            continue
        print("%-24s %6d | mean=%6.1f min=%4s max=%4s | mean=%6.1f min=%4s max=%4s | %+.1f us" % (
            label, len(fs),
            arb._mean(firsts), min(firsts) if firsts else "-", max(firsts) if firsts else "-",
            arb._mean(rest), min(rest) if rest else "-", max(rest) if rest else "-",
            arb._mean(firsts) - arb._mean(rest)))


def score(path, want=DEFAULT_FRAME):
    windows = []
    trials = []
    frames = []
    with open(path) as fh:
        for line in fh:
            line = line.rstrip("\n")
            if line.startswith("#"):
                if " ours trial=" in line:
                    kv = dict(p.split("=", 1) for p in line.split() if "=" in p)
                    t0, t1 = float(kv["t0"]), float(kv["t1"])
                    windows.append((t0, t1 + ATTRIB_WINDOW_S))
                    trials.append(kv)
                continue
            parts = line.split(" ", 1)
            if len(parts) != 2:
                continue
            try:
                host_t = float(parts[0])
            except ValueError:
                continue
            f = arb.parse_line(parts[1], host_t)
            if f:
                frames.append(f)

    ours, ref, backward, other = [], [], [], []
    for f in frames:
        b, _ = arb.decode_bytes(f)
        inside = any(t0 <= f.host_t <= t1 for (t0, t1) in windows)
        if b is not None and len(b) == 2:
            (ours if inside else ref).append(f)
        elif b is not None and len(b) == 1:
            backward.append(f)
        else:
            other.append(f)

    print("file: %s" % path)
    _first_pulse_table(ours, ref, backward)
    print("trials=%d  frames=%d  ours=%d  reference=%d  backward=%d  other=%d\n"
          % (len(trials), len(frames), len(ours), len(ref), len(backward), len(other)))
    a = arb.summarize(ours, "OUR TX (dali2rust P4)")
    b = arb.summarize(ref, "REFERENCE TX (WB)")
    arb.summarize(backward, "GEAR backward frames")

    if a["low1"] and b["low1"]:
        print("\ndelta ours-reference: low1=%+.1f us  high1=%+.1f us" %
              (arb._mean(a["low1"]) - arb._mean(b["low1"]),
               arb._mean(a["high1"]) - arb._mean(b["high1"])))
    return ours, ref, backward


def main(argv):
    if len(argv) > 1 and argv[1] == "score":
        score(argv[2])
        return 0
    port = argv[1] if len(argv) > 1 else "/dev/cu.usbmodem11401"
    out = argv[2] if len(argv) > 2 else "runs/arbiter/ab.txt"
    trials = int(argv[3]) if len(argv) > 3 else 60
    res = run(port, out, trials=trials)
    ok = [r for r in res if r["n_match"] == 1]
    print("\n%d/%d trials attributed unambiguously -> %s" % (len(ok), len(res), out))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
