import re
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hil import config as config_mod
from hil import serialmon
from hil.api import Client

TIMING = re.compile(r"DALI sniff timing:.*stage max=(\d+) ticks over budget=(\d+) of (\d+) "
                    r"poll gap max=(\d+) us")
FLUSH = re.compile(r"slow flush_dirty_slices took (\d+) ms")
REDUNDANCY_FIELDS = ("answered", "window_closed", "late", "suppressed", "aborted")
DEFAULT_PHASE_S = 150
WRITE_EVERY_S = 1.5
QUIET_POLL_S = 2.0
SETTLE_S = 6.0
MARKER = "hil-flush-probe"


def counters(api):
    diag = api.diagnostics()
    redundancy, sniffer = diag["redundancy"], diag["phy_sniffer"]
    out = {key: redundancy[key] for key in REDUNDANCY_FIELDS}
    out.update(failed=sniffer["decode_failed"], unsup=sniffer["unsupported_len"],
               frames=sniffer["frames"])
    return out


def log_size(path):
    return path.stat().st_size if path.exists() else 0


def log_lines(path, start):
    if not path.exists() or path.stat().st_size <= start:
        return None
    with open(path, "rb") as fh:
        fh.seek(start)
        return fh.read().decode("utf-8", "replace").splitlines()


def window(lines):
    stages, overs, totals, gaps, flushes = [], 0, 0, [], []
    for line in lines:
        timing = TIMING.search(line)
        if timing:
            stages.append(int(timing.group(1)))
            overs += int(timing.group(2))
            totals += int(timing.group(3))
            gaps.append(int(timing.group(4)))
            continue
        flush = FLUSH.search(line)
        if flush:
            flushes.append(int(flush.group(1)))
    return stages, overs, totals, gaps, flushes


class NotesToggle:
    def __init__(self, api, short):
        self.api, self.short = api, short
        self.original = api.state(short).get("notes")
        self.marked = False

    def __call__(self):
        self.marked = not self.marked
        self.api.device_patch(self.short,
                              {"notes": MARKER if self.marked else self.original})

    def restore(self):
        if self.api.state(self.short).get("notes") != self.original:
            self.api.device_patch(self.short, {"notes": self.original})


def probe_target(api, cfg):
    allowed = cfg.lamp_short_set()
    shorts = [d["short_address"] for d in api.devices()["physical_devices"]
              if d["short_address"] in allowed]
    if not shorts:
        raise SystemExit("no gear in HIL_LAMP_SHORTS=%s is registered — nothing this "
                         "script may write" % cfg.lamp_shorts)
    return shorts[0]


def _drive(seconds, provoke):
    writes, failures = 0, 0
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if provoke is None:
            time.sleep(QUIET_POLL_S)
            continue
        try:
            provoke()
            writes += 1
        except Exception as exc:
            failures += 1
            print("  write failed: %r" % exc)
        time.sleep(WRITE_EVERY_S)
    return writes, failures


def _report(name, seconds, writes, delta, lines):
    print("\n=== %s (%ds, %d writes) ===" % (name, seconds, writes))
    print("  probes answered %(answered)d, window_closed %(window_closed)d, late %(late)d, "
          "suppressed %(suppressed)d, aborted %(aborted)d" % delta)
    print("  bad captures: failed %(failed)d, unsup %(unsup)d  (frames %(frames)d)" % delta)
    stages, overs, totals, gaps, flushes = window(lines)
    if stages:
        stages.sort()
        gaps.sort()
        print("  stage ticks: p50 %d max %d | over budget %d of %d"
              % (stages[len(stages) // 2], max(stages), overs, totals))
        print("  poll gap us: p50 %d max %d" % (gaps[len(gaps) // 2], max(gaps)))
    print("  slow flushes (>250ms): %d  total %d ms  max %d ms"
          % (len(flushes), sum(flushes), max(flushes) if flushes else 0))


def phase(api, log, name, seconds, provoke=None):
    before, start = counters(api), log_size(log)
    writes, failures = _drive(seconds, provoke)
    after = counters(api)
    time.sleep(SETTLE_S)
    lines = log_lines(log, start)
    if lines is None:
        print("\n=== %s: serial window EMPTY — %s did not grow, no evidence ===" % (name, log))
        return False
    _report(name, seconds, writes, {k: after[k] - before[k] for k in before}, lines)
    return failures == 0


def main(argv):
    seconds = int(argv[1]) if len(argv) > 1 else DEFAULT_PHASE_S
    cfg = config_mod.load()
    api = Client(cfg)
    log = serialmon.log_path(cfg)
    toggle = NotesToggle(api, probe_target(api, cfg))
    try:
        quiet = phase(api, log, "A  QUIET (no writes)", seconds)
        provoked = phase(api, log, "B  PROVOKED (SA%d notes rewritten every %.1f s)"
                         % (toggle.short, WRITE_EVERY_S), seconds, toggle)
    finally:
        toggle.restore()
    return 0 if quiet and provoked else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
