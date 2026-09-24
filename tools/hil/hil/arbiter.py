import re
import sys
import time
from dataclasses import dataclass, field

USAGE = """usage: python3 -m hil.arbiter capture PORT SECONDS OUT [--echo]
       python3 -m hil.arbiter summary CAPTURE
       python3 -m hil.arbiter dump CAPTURE

Records the RMT wire arbiter's raw pulse lines and decodes them on the host."""

HALF_BIT_US = 416.67
BIT_US = 833.33
RX_HALF_BIT_MIN_US = 333.33
RX_HALF_BIT_MAX_US = 500.0
RX_DOUBLE_MIN_US = 666.67
RX_DOUBLE_MAX_US = 1000.0

SINGLE_DOUBLE_SPLIT_US = 625.0

_LINE = re.compile(r"^E (\d+) (-?\d+) (-?\d+)((?: \d+)*)\s*$")


@dataclass
class Frame:
    seq: int
    t_end_us: int
    first_level: int
    durations: list = field(default_factory=list)
    host_t: float = 0.0

    @property
    def span_us(self):
        return sum(self.durations)

    @property
    def t_start_us(self):
        return self.t_end_us - self.span_us - IDLE_END_US


IDLE_END_US = 2000


def parse_line(line, host_t=0.0):
    m = _LINE.match(line.strip())
    if not m:
        return None
    seq, t_end, lvl, rest = m.groups()
    durations = [int(x) for x in rest.split()] if rest.strip() else []
    while durations and durations[-1] == 0:
        durations.pop()
    return Frame(int(seq), int(t_end), int(lvl), durations, host_t)


def pulses(frame):
    lvl = frame.first_level
    out = []
    for d in frame.durations:
        out.append((lvl, d))
        lvl ^= 1
    return out


def half_bit_stream(frame):
    out = []
    for lvl, d in pulses(frame):
        if d < SINGLE_DOUBLE_SPLIT_US:
            count = 1
        elif d < 1.5 * BIT_US:
            count = 2
        else:
            count = 0
        out.append((lvl, d, count))
    return out


def decode(frame, invert=False):
    slots = []
    for lvl, _d, count in half_bit_stream(frame):
        if count == 0:
            return None, "unclassified pulse"
        eff = lvl ^ (1 if invert else 0)
        slots.extend([eff] * count)

    if len(slots) % 2:
        slots.append(1)
    bits = []
    for i in range(0, len(slots), 2):
        a, b = slots[i], slots[i + 1]
        if a == 0 and b == 1:
            bits.append(1)
        elif a == 1 and b == 0:
            bits.append(0)
        else:
            return None, "non-manchester pair at bit %d" % (i // 2)
    if not bits or bits[0] != 1:
        return None, "no start bit"
    return bits[1:], None


def decode_bytes(frame, invert=False):
    bits, err = decode(frame, invert)
    if bits is None:
        return None, err
    if len(bits) not in (8, 16, 24):
        return None, "odd bit count %d" % len(bits)
    out = bytearray()
    for i in range(0, len(bits), 8):
        v = 0
        for b in bits[i:i + 8]:
            v = (v << 1) | b
        out.append(v)
    return bytes(out), None


def classify(frame):
    buckets = {"low1": [], "high1": [], "low2": [], "high2": []}
    for lvl, d, count in half_bit_stream(frame):
        if count == 0:
            continue
        key = ("low" if lvl == 0 else "high") + str(count)
        buckets[key].append(d)
    return buckets


def capture(port, seconds, out_path, baud=115200, echo=False):
    import serial

    s = serial.Serial(port, baud, timeout=0.2)
    t0 = time.time()
    started = time.monotonic()
    n = 0
    with open(out_path, "w") as fh:
        fh.write("# arbiter capture start %.3f port=%s\n" % (t0, port))
        buf = b""
        while time.monotonic() - started < seconds:
            buf += s.read(8192)
            while b"\n" in buf:
                raw, buf = buf.split(b"\n", 1)
                line = raw.decode("utf-8", "replace").rstrip("\r")
                if not line:
                    continue
                fh.write("%.6f %s\n" % (time.time(), line))
                if line.startswith("E "):
                    n += 1
                if echo:
                    print(line)
            fh.flush()
    s.close()
    return n


def load(path):
    frames = []
    with open(path) as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split(" ", 1)
            host_t = 0.0
            if len(parts) == 2 and parts[0].replace(".", "").isdigit():
                host_t, line = float(parts[0]), parts[1]
            f = parse_line(line, host_t)
            if f:
                frames.append(f)
    return frames


def _mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


def summarize(frames, label=""):
    agg = {"low1": [], "high1": [], "low2": [], "high2": []}
    decoded = 0
    for f in frames:
        b, _ = decode_bytes(f)
        if b is not None:
            decoded += 1
        for k, v in classify(f).items():
            agg[k].extend(v)
    print("== %s: %d frames, %d decoded ==" % (label, len(frames), decoded))
    for k in ("low1", "high1", "low2", "high2"):
        v = agg[k]
        if not v:
            continue
        print("  %-6s n=%-5d mean=%7.1f min=%5d max=%5d" %
              (k, len(v), _mean(v), min(v), max(v)))
    if agg["low1"] and agg["high1"]:
        print("  duty asymmetry (high1-low1) = %.1f us" %
              (_mean(agg["high1"]) - _mean(agg["low1"])))
    return agg


def main(argv):
    if len(argv) < 2:
        print(USAGE)
        return 2
    cmd = argv[1]
    if cmd == "capture":
        port = argv[2]
        seconds = float(argv[3])
        out = argv[4]
        n = capture(port, seconds, out, echo="--echo" in argv)
        print("captured %d frames -> %s" % (n, out))
        return 0
    if cmd == "summary":
        frames = load(argv[2])
        summarize(frames, argv[2])
        return 0
    if cmd == "dump":
        frames = load(argv[2])
        prev_end = None
        for f in frames:
            b, err = decode_bytes(f)
            gap = "" if prev_end is None else " gap=%.1fms" % ((f.t_start_us - prev_end) / 1000.0)
            prev_end = f.t_end_us - IDLE_END_US
            hexs = b.hex() if b else "-"
            bk = classify(f)
            print("seq=%-5d t=%-12d n=%-3d %-8s %-22s lo1=%5.1f hi1=%5.1f span=%dus%s" % (
                f.seq, f.t_start_us, len(f.durations), hexs, err or "",
                _mean(bk["low1"]), _mean(bk["high1"]), f.span_us, gap))
        return 0
    print("unknown command %r" % cmd)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
