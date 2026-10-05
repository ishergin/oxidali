import re
import sys
from collections import defaultdict

SET_DTR1 = 0xC3
SET_DTR0 = 0xA3
READ_MEMORY_LOCATION = 0xC5
QUERY_CONTENT_DTR1 = 0x9C
HEADER_LOCATIONS = 3

FORWARD = re.compile(r"forward16 0x([0-9a-f]{4})")
BACKWARD = re.compile(r"backward decode -> (Some\((\d+)\)|None)")
STAMP = re.compile(r"^(\d{4}-\d\d-\d\d)")

DEFAULT_LOG = "state/persist/serial.log"


def is_addressed(byte):
    return byte & 0x01 and byte < 0x80


def audit(path):
    dtr1 = dtr0 = None
    pending = None
    day = None
    headers = defaultdict(lambda: defaultdict(int))
    header_day = defaultdict(lambda: defaultdict(int))
    arm = defaultdict(lambda: defaultdict(int))
    open_probe = {}

    with open(path, errors="replace") as handle:
        for line in handle:
            stamped = STAMP.match(line)
            if stamped:
                day = stamped.group(1)
            forward = FORWARD.search(line)
            if forward:
                frame = int(forward.group(1), 16)
                high, low = frame >> 8, frame & 0xFF
                if high == SET_DTR1:
                    dtr1, dtr0 = low, None
                elif high == SET_DTR0:
                    dtr0 = low
                elif low == READ_MEMORY_LOCATION and is_addressed(high):
                    pending = ("read", high, dtr1, dtr0, day)
                elif low == QUERY_CONTENT_DTR1 and is_addressed(high):
                    pending = ("arm", high, dtr1, None, day)
                else:
                    pending = None
                continue

            backward = BACKWARD.search(line)
            if not (backward and pending):
                continue
            kind, address, bank, offset, stamp = pending
            value = int(backward.group(2)) if backward.group(2) else None
            pending = None

            if kind == "arm":
                if bank is None:
                    continue
                verdict = ("no answer" if value is None
                           else "latched" if value == bank else "stale")
                arm[stamp][verdict] += 1
                continue

            if bank is not None and offset is not None:
                if offset == 0:
                    open_probe[(address, bank)] = (value, stamp)
                elif offset == 2 and (address, bank) in open_probe:
                    loc0, probe_day = open_probe.pop((address, bank))
                    headers[bank][(loc0, value)] += 1
                    header_day[(bank, probe_day)][(loc0, value)] += 1
            if offset is not None:
                dtr0 = offset + 1
    return headers, header_day, arm


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_LOG
    headers, header_day, arm = audit(path)

    print("Header probes by the bank we selected — (last-address, location 2):")
    for bank in sorted(headers):
        pairs = sorted(headers[bank].items(), key=lambda kv: -kv[1])
        rendered = ", ".join("(%s,%s)x%d" % (p[0], p[1], n) for p, n in pairs[:6])
        print("  bank %d  n=%-5d %s" % (bank, sum(headers[bank].values()), rendered))

    print("\nBank-0 probes per day (a pair that belongs to another bank is stale):")
    days = sorted({day for bank, day in header_day if bank == 0})
    for day in days:
        counts = header_day[(0, day)]
        total = sum(counts.values())
        foreign = sum(n for (loc0, loc2), n in counts.items() if loc2 != 1)
        print("  %s  probes=%-5d foreign=%d" % (day, total, foreign))

    print("\n`QUERY CONTENT DTR1` verdicts per day (the arm proving the bank):")
    for day in sorted(arm):
        counts = arm[day]
        print("  %s  latched=%-6d stale=%-5d no-answer=%d"
              % (day, counts["latched"], counts["stale"], counts["no answer"]))


if __name__ == "__main__":
    main()
