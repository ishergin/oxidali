import re

from hil.lamp_guard import (BROADCAST_FIRST, GROUP_MASK, TARGET_SEGMENT, describe_frame,
                            frame_writes, wire_target)

TX_LINE = re.compile(r"DALI PHY TX: forward16 0x([0-9a-f]{4}) \(ISR-owned bitbang\)")
BYTE_BITS = 8
BYTE = 0xFF
LOG_LOSS_COUNTERS = ("console_log_dropped_total", "console_log_busy_total",
                     "console_log_truncated_total", "console_log_unavailable_total")


def sent_frames(lines):
    out = []
    for line in lines:
        match = TX_LINE.search(line)
        if match:
            frame = int(match.group(1), 16)
            out.append((frame >> BYTE_BITS, frame & BYTE))
    return out


def violations(lines, park, groups):
    park, groups, out = frozenset(park), frozenset(groups), []
    for addr, data in sent_frames(lines):
        target = wire_target(addr)
        if target is None or not frame_writes(addr, data):
            continue
        if addr >= BROADCAST_FIRST:
            out.append("broadcast on a shared wire: %s" % describe_frame(addr, data))
        elif target == TARGET_SEGMENT and (addr >> 1) & GROUP_MASK not in groups:
            out.append("a group outside the session: %s" % describe_frame(addr, data))
        elif target != TARGET_SEGMENT and target not in park:
            out.append("a lamp outside the park: %s" % describe_frame(addr, data))
    return out


def log_losses(stats):
    dali = stats.get("dali") or {}
    return {name: dali.get(name, 0) for name in LOG_LOSS_COUNTERS}


def lost_lines(before, after):
    return {name: after[name] - before[name] for name in before
            if after.get(name, 0) != before.get(name, 0)}
