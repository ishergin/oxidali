import argparse
import re
import sys

from hil import config

DESCRIPTION = 'How long a discovery scan actually spends on the wire, from the serial log.'

LINE = re.compile(r"^(\S+) I \((\d+)\)")
SCAN_POST = "POST /api/v1/adapters/0/discovery-runs"
SHORT_63_PROBE = "PHY TX: forward16 0x7f91"


def _percentile(values, pct):
    ordered = sorted(values)
    idx = int(round(pct / 100.0 * (len(ordered) - 1)))
    return ordered[min(idx, len(ordered) - 1)]


def _collect(path, since):
    rows = []
    with open(path, errors="replace") as fh:
        for line in fh:
            if since and not line.startswith(since):
                continue
            if "DALI PHY" not in line and SCAN_POST not in line:
                continue
            match = LINE.match(line)
            if not match:
                continue
            host, uptime = match.group(1), int(match.group(2))
            if SCAN_POST in line:
                rows.append(("SCAN", host, uptime))
            elif SHORT_63_PROBE in line:
                rows.append(("SWEEP_END", host, uptime))
            elif "PHY TX" in line:
                rows.append(("TX", host, uptime))
    return rows


def _one_scan(rows, start, gap_ms):
    _, host, first = rows[start]
    stamps, sweep_end = [], None
    previous = first
    for kind, _, uptime in rows[start + 1:]:
        if kind == "SCAN" or uptime < previous - 1000 or uptime - previous > gap_ms:
            break
        previous = uptime
        if kind == "SWEEP_END" and sweep_end is None:
            sweep_end = uptime - first
        stamps.append(uptime)
    return host, first, stamps, sweep_end


def report(path, since, gap_ms, widened_gap_ms):
    rows = _collect(path, since)
    starts = [i for i, row in enumerate(rows) if row[0] == "SCAN"]
    if not starts:
        print("no discovery runs in %s%s" % (path, " since " + since if since else ""))
        return 1
    print("%-26s %8s %6s %8s  %s" % ("scan start", "wire", "frames", "to sh63",
                                     "frame gap ms (p50/p90/p99/max)"))
    for start in starts:
        host, first, stamps, sweep_end = _one_scan(rows, start, gap_ms)
        if len(stamps) < 2:
            print("%-26s %8s %6d" % (host, "-", len(stamps)))
            continue
        gaps = [b - a for a, b in zip(stamps, stamps[1:])]
        _, _, wide, _ = _one_scan(rows, start, widened_gap_ms)
        drift = "" if len(wide) == len(stamps) else \
            "   (!! %d frames at gap %ds — widen and re-read)" % (len(wide), widened_gap_ms / 1000)
        print("%-26s %7.1fs %6d %7s  %d/%d/%d/%d%s" % (
            host, (stamps[-1] - first) / 1000.0, len(stamps),
            "%.1fs" % (sweep_end / 1000.0) if sweep_end else "-",
            _percentile(gaps, 50), _percentile(gaps, 90),
            _percentile(gaps, 99), max(gaps), drift))
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument("--log", default=None, help="serial log (default: the persistent monitor's)")
    parser.add_argument("--since", default=None, help="only lines starting with this prefix, e.g. 2026-08-18")
    parser.add_argument("--gap-ms", type=int, default=4000, help="quiet gap that ends a scan (default 4000)")
    parser.add_argument("--widened-gap-ms", type=int, default=30000,
                        help="second threshold used only to flag an unstable answer (default 30000)")
    args = parser.parse_args(argv)
    path = args.log or str(config.load().persist_serial_log)
    return report(path, args.since, args.gap_ms, args.widened_gap_ms)


if __name__ == "__main__":
    sys.exit(main())
