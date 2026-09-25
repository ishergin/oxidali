import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

from hil.pidfile import pid_alive, sidecar_log

SSH_OPTS = [
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=5",
    "-o", "ControlMaster=auto",
    "-o", "ControlPath=/tmp/hil-ssh-%r@%h:%p",
    "-o", "ControlPersist=60",
]


def ssh_argv(cfg, remote_cmd):
    return ["ssh", *SSH_OPTS, cfg.wb_ssh, remote_cmd]

WIRE_BYTE0_IS_LSB = False

RETAINED_WINDOW_S = 0.7

SPECIAL = {
    0xA1: "TERMINATE", 0xA3: "DTR0", 0xA5: "INITIALISE", 0xA7: "RANDOMISE",
    0xA9: "COMPARE", 0xAB: "WITHDRAW", 0xAD: "PING (DALI-2)",
    0xB1: "SEARCHADDRH", 0xB3: "SEARCHADDRM",
    0xB5: "SEARCHADDRL", 0xB7: "PROGRAM SHORT ADDRESS", 0xB9: "VERIFY SHORT ADDRESS",
    0xBB: "QUERY SHORT ADDRESS", 0xC1: "ENABLE DEVICE TYPE", 0xC3: "DTR1", 0xC5: "DTR2",
    0xC7: "WRITE MEMORY LOCATION", 0xC9: "WRITE MEMORY LOCATION NO REPLY",
}

COMMANDS = {
    0x00: "OFF", 0x01: "UP", 0x02: "DOWN", 0x03: "STEP UP", 0x04: "STEP DOWN",
    0x05: "RECALL MAX", 0x06: "RECALL MIN", 0x07: "STEP DOWN AND OFF",
    0x08: "ON AND STEP UP", 0x0A: "GO TO LAST ACTIVE LEVEL",
    0x20: "RESET", 0x21: "STORE ACTUAL LEVEL IN DTR0",
    0x25: "IDENTIFY DEVICE",
    0x2A: "STORE DTR AS MAX LEVEL", 0x2B: "STORE DTR AS MIN LEVEL",
    0x2C: "STORE DTR AS SYSTEM FAILURE LEVEL", 0x2D: "STORE DTR AS POWER ON LEVEL",
    0x2E: "STORE DTR AS FADE TIME", 0x2F: "STORE DTR AS FADE RATE",
    0x80: "STORE DTR AS SHORT ADDRESS", 0x81: "ENABLE WRITE MEMORY",
    0x90: "QUERY STATUS", 0x91: "QUERY CONTROL GEAR PRESENT", 0x92: "QUERY LAMP FAILURE",
    0x93: "QUERY LAMP POWER ON", 0x94: "QUERY LIMIT ERROR", 0x95: "QUERY RESET STATE",
    0x96: "QUERY MISSING SHORT ADDRESS", 0x97: "QUERY VERSION NUMBER",
    0x98: "QUERY CONTENT DTR0", 0x99: "QUERY DEVICE TYPE", 0x9A: "QUERY PHYSICAL MINIMUM",
    0x9B: "QUERY POWER FAILURE", 0xA0: "QUERY ACTUAL LEVEL", 0xA1: "QUERY MAX LEVEL",
    0xA2: "QUERY MIN LEVEL", 0xA3: "QUERY POWER ON LEVEL", 0xA4: "QUERY SYSTEM FAILURE LEVEL",
    0xA5: "QUERY FADE TIME/FADE RATE", 0xC0: "QUERY GROUPS 0-7", 0xC1: "QUERY GROUPS 8-15",
    0xC2: "QUERY RANDOM ADDRESS H", 0xC3: "QUERY RANDOM ADDRESS M", 0xC4: "QUERY RANDOM ADDRESS L",
    0xC5: "READ MEMORY LOCATION",
    0xE0: "DT8 SET TEMP X-COORD", 0xE1: "DT8 SET TEMP Y-COORD", 0xE2: "DT8 ACTIVATE",
    0xE7: "DT8 SET TEMP COLOUR TEMPERATURE", 0xE8: "DT8 CCT STEP COOLER",
    0xE9: "DT8 CCT STEP WARMER", 0xEB: "DT8 SET TEMP RGB DIM LEVEL",
    0xF3: "DT8 STORE GEAR FEATURES/STATUS",
    0xF7: "DT8 QUERY GEAR FEATURES/STATUS",
    0xF8: "DT8 QUERY COLOUR STATUS", 0xFA: "DT8 QUERY COLOUR VALUE",
}


def wire_bytes(data, nbits):
    n = max(1, nbits // 8)
    stored = [(data >> (8 * i)) & 0xFF for i in range(n)]
    return stored if WIRE_BYTE0_IS_LSB else list(reversed(stored))


def address_name(a):
    if a & 0x80 == 0:
        return "short %d" % (a >> 1)
    if (a & 0xE0) == 0x80:
        return "group %d" % ((a >> 1) & 0x0F)
    if a in (0xFE, 0xFF):
        return "broadcast"
    if a in (0xFC, 0xFD):
        return "broadcast-unaddressed"
    return None


def decode_forward16(b):
    a, c = b[0], b[1]
    tgt = address_name(a)
    if tgt is None:
        name = SPECIAL.get(a, "special 0x%02X" % a)
        return "%s data=0x%02X" % (name, c)
    if a & 0x01 == 0:
        return "DAPC %s -> level %d" % (tgt, c)
    if 0x10 <= c <= 0x1F:
        return "GO TO SCENE %d (%s)" % (c - 0x10, tgt)
    if 0x40 <= c <= 0x4F:
        return "STORE DTR AS SCENE %d (%s)" % (c - 0x40, tgt)
    if 0x50 <= c <= 0x5F:
        return "REMOVE FROM SCENE %d (%s)" % (c - 0x50, tgt)
    if 0x60 <= c <= 0x6F:
        return "ADD TO GROUP %d (%s)" % (c - 0x60, tgt)
    if 0x70 <= c <= 0x7F:
        return "REMOVE FROM GROUP %d (%s)" % (c - 0x70, tgt)
    if 0xB0 <= c <= 0xBF:
        return "QUERY SCENE LEVEL %d (%s)" % (c - 0xB0, tgt)
    name = COMMANDS.get(c, "cmd 0x%02X" % c)
    return "%s (%s)" % (name, tgt)


def parse_monitor_line(ts, payload):
    payload = payload.strip()
    if payload.startswith(">>"):
        direction = "tx"
    elif payload.startswith("<<"):
        direction = "rx"
    else:
        return None
    text = payload[2:].strip()
    fc = None
    m = re.search(r"\(fc: (\d+)\)", text)
    if m:
        fc = int(m.group(1))
    return {"kind": "monitor", "ts": ts, "direction": direction,
            "text": text, "fc": fc,
            "lunatone": "(from lunatone)" in text}


def parse_line(line):
    parts = line.strip().split(None, 2)
    if len(parts) == 3:
        ts, topic, payload = parts
    elif len(parts) == 2:
        ts, (topic, payload) = None, parts
    else:
        return None
    try:
        tsf_monitor = float(ts) if ts is not None else None
    except ValueError:
        return None
    if topic.endswith("/bus_monitor"):
        return parse_monitor_line(tsf_monitor, payload)
    if "monitor_sporadic_frame" not in topic:
        return None
    payload = payload.split()[0]
    try:
        v = int(payload)
        tsf = float(ts) if ts is not None else None
    except ValueError:
        return None
    nbits = (v >> 32) & 0xFF
    flags = (v >> 40) & 0xFF
    b = wire_bytes(v & 0xFFFFFFFF, nbits)
    frame = {
        "ts": tsf,
        "slot": topic.rsplit("_", 1)[-1],
        "raw": v,
        "counter": (v >> 48) & 0xFFFF,
        "len_bits": nbits,
        "backward": bool(flags & 0x01),
        "corrupted": bool(flags & 0x02),
        "bytes": "".join("%02X" % x for x in b),
    }
    if frame["corrupted"]:
        frame["decoded"] = "CORRUPTED"
    elif frame["backward"] or nbits == 8:
        frame["decoded"] = "backward 0x%02X (%d)" % (b[0], b[0])
    elif nbits == 16:
        frame["decoded"] = decode_forward16(b)
    elif nbits == 24:
        frame["decoded"] = "forward24 (DALI-2 device frame)"
    else:
        frame["decoded"] = "len=%d bits" % nbits
    return frame


def load(stream):
    frames, seen = [], set()
    for line in stream:
        f = parse_line(line)
        if f is None or f.get("kind") == "monitor":
            continue
        key = (f["counter"], f["raw"])
        if key in seen:
            continue
        seen.add(key)
        frames.append(f)
    t0 = min((f["ts"] for f in frames if f["ts"]), default=None)
    for f in frames:
        f["retained"] = bool(t0 is not None and f["ts"] is not None
                             and f["ts"] - t0 < RETAINED_WINDOW_S)
    return frames


def counter_gaps(frames):
    counters = sorted({f["counter"] for f in frames})
    if len(counters) < 2:
        return 0, 0
    missed = 0
    worst = 0
    for prev, cur in zip(counters, counters[1:]):
        d = (cur - prev) % 65536
        if d > 1:
            missed += d - 1
            worst = max(worst, d - 1)
    return missed, worst


def stats(frames):
    live = [f for f in frames if not f["retained"]]
    tss = [f["ts"] for f in live if f["ts"]]
    span = (max(tss) - min(tss)) if len(tss) >= 2 else 0.0
    missed, worst = counter_gaps(live)
    return {
        "frames_total": len(frames),
        "frames_live": len(live),
        "frames_retained_snapshot": len(frames) - len(live),
        "backward": sum(1 for f in live if f["backward"]),
        "corrupted": sum(1 for f in live if f["corrupted"]),
        "counter_missed": missed,
        "counter_worst_gap": worst,
        "span_seconds": round(span, 3),
        "rate_hz": round(len(live) / span, 2) if span > 0 else None,
    }


def _topics(cfg):
    topics = [t for i in (1, 2, 3, 4) for t in
              ("-t", "/devices/%s/controls/bus_%d_monitor_sporadic_frame_%d"
               % (cfg.wb_device, cfg.wb_bus, i))]
    topics += ["-t", "/wb-dali/%s_bus_%d/bus_monitor"
               % (cfg.wb_device, cfg.wb_bus)]
    return topics


def _remote_cmd(cfg, prefix=""):
    topics = " ".join(_topics(cfg))
    return ('%s stdbuf -oL mosquitto_sub -v %s | '
            'while IFS= read -r l; do echo "$(date +%%s.%%3N) $l"; done'
            % (prefix, topics)).strip()


class SnifferTap:
    def __init__(self, cfg, log_path=None):
        self.cfg = cfg
        self._pidfile = Path(cfg.state_dir) / "sniffer_tap.pid"
        self._proc = None
        self.owned = False
        self.witness_fallbacks = 0
        attached = self._attach_existing()
        if attached:
            self.log_path = attached
        else:
            self.log_path = Path(log_path or (cfg.run_dir() / "sniffer.log"))
            self._start()

    def _attach_existing(self):
        if not pid_alive(self._pidfile):
            return None
        return sidecar_log(self._pidfile)

    def _start(self):
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        self._log_fh = open(self.log_path, "ab")
        self._proc = subprocess.Popen(
            ssh_argv(self.cfg, _remote_cmd(self.cfg)),
            stdout=self._log_fh, stderr=subprocess.DEVNULL,
            start_new_session=True)
        self.owned = True

    @property
    def alive(self):
        if self._proc is not None:
            return self._proc.poll() is None
        return self._attach_existing() is not None

    def wait_ready(self, timeout_s=8.0):
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if self.log_path.exists() and self.log_path.stat().st_size > 0:
                return True
            if not self.alive:
                return False
            time.sleep(0.3)
        return False

    def window(self):
        return Window(self)

    def close(self):
        if self._proc is not None and self.owned:
            try:
                os.killpg(os.getpgid(self._proc.pid), signal.SIGTERM)
            except OSError:
                self._proc.terminate()
            self._proc.wait(timeout=5)
            self._log_fh.close()
            subprocess.run(
                ssh_argv(self.cfg,
                         'pkill -f "mosquitto_sub.*monitor_sporadic" 2>/dev/null; true'),
                capture_output=True, timeout=10)


class Window:
    def __init__(self, tap: SnifferTap):
        self.tap = tap
        self._offset = 0
        self.expected = []

    def __enter__(self):
        self._offset = self.tap.log_path.stat().st_size if self.tap.log_path.exists() else 0
        return self

    def __exit__(self, *exc):
        return False

    def _entries(self):
        if not self.tap.log_path.exists():
            return []
        with open(self.tap.log_path) as fh:
            fh.seek(self._offset)
            return [f for f in map(parse_line, fh) if f is not None]

    def frames(self):
        out = []
        for f in self._entries():
            if f.get("kind") == "monitor":
                continue
            f["retained"] = False
            out.append(f)
        out.sort(key=lambda f: ((f["ts"] or 0.0), f["counter"]))
        return out

    def monitor_lines(self):
        return [f for f in self._entries() if f.get("kind") == "monitor"]

    def expect_monitor(self, contains, direction=None, timeout_s=10.0,
                       resend=None):
        deadline = time.monotonic() + timeout_s
        resend_at = time.monotonic() + timeout_s / 2.0 if resend else None
        while True:
            for f in self.monitor_lines():
                if direction and f["direction"] != direction:
                    continue
                if contains in f["text"]:
                    return f
            now = time.monotonic()
            if resend_at and now >= resend_at:
                resend()
                resend_at = None
            if now >= deadline:
                raise AssertionError(
                    "bus-monitor: no %s line containing %r within %.0fs; saw: %s"
                    % (direction or "any", contains, timeout_s,
                       [f["text"] for f in self.monitor_lines()][-8:]))
            time.sleep(0.4)

    def expect_frame(self, contains, timeout_s=10.0, resend=None):
        self.expected.append(contains)
        deadline = time.monotonic() + timeout_s
        resend_at = time.monotonic() + timeout_s / 2.0 if resend else None
        while True:
            for f in self.frames():
                if contains in f["decoded"]:
                    return f
            now = time.monotonic()
            if resend_at and now >= resend_at:
                resend()
                resend_at = None
            if now >= deadline:
                raise AssertionError(
                    "sniffer: no frame containing %r within %.0fs; saw: %s"
                    % (contains, timeout_s,
                       [f["decoded"] for f in self.frames()][-8:]))
            time.sleep(0.4)

    def expect_program_witness(self, primary, readback, timeout_s=10.0):
        try:
            witness = self.expect_frame(primary, timeout_s=timeout_s), "frame"
        except AssertionError:
            self.tap.witness_fallbacks += 1
            return self.expect_frame(readback, timeout_s=2.0), "readback"
        return witness

    def expect_quiet(self, contains, settle_s=3.0):
        time.sleep(settle_s)
        hits = [f for f in self.frames() if contains in f["decoded"]]
        if hits:
            raise AssertionError("sniffer: unexpected frame(s) %r: %s"
                                 % (contains, [f["decoded"] for f in hits][:5]))

    def foreign_frames(self):
        out = []
        for f in self.frames():
            if f["backward"] or f["corrupted"]:
                continue
            if any(e in f["decoded"] for e in self.expected):
                continue
            out.append(f)
        return out

    @property
    def contaminated(self):
        return len(self.foreign_frames()) > 0

    def stats(self):
        return stats(self.frames())


def decode_main(argv=None, parser_class=argparse.ArgumentParser):
    ap = parser_class(prog="hil decode", description="decode WB sporadic-frame captures")
    ap.add_argument("logfile", nargs="?", help="tap log (default: stdin)")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--keep-retained", action="store_true",
                    help="include the initial retained-slot snapshot in output")
    args = ap.parse_args(argv)

    stream = open(args.logfile) if args.logfile else sys.stdin
    frames = load(stream)
    frames.sort(key=lambda f: ((f["ts"] or 0.0), f["counter"]))

    if args.stats:
        s = stats(frames)
        print(json.dumps(s) if args.json else
              "\n".join("%s: %s" % kv for kv in s.items()))
        return

    shown = frames if args.keep_retained else [f for f in frames if not f["retained"]]
    prev_counter = None
    for f in shown:
        if args.json:
            print(json.dumps(f))
            continue
        gap = ""
        if prev_counter is not None:
            d = (f["counter"] - prev_counter) % 65536
            if d > 1:
                gap = "  !! %d frame(s) missed" % (d - 1)
        prev_counter = f["counter"]
        ts = ("%.3f" % f["ts"]) if f["ts"] else "-"
        kind = "BWD" if f["backward"] else "F%02d" % f["len_bits"]
        print("%s #%05d [%s] %-8s %s%s"
              % (ts, f["counter"], kind, f["bytes"], f["decoded"], gap))
