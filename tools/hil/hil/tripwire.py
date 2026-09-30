import re

from hil.lamp_guard import LampNotAllowed, describe_frame, enables
from hil.wait import settled

TX_LINE = re.compile(
    r"DALI PHY TX: forward16 0x([0-9a-f]{4})(?: \(batched\))? \(ISR-owned bitbang\)")
COLLISION_LINE = re.compile(r"DALI PHY TX: collision on forward16 0x([0-9a-f]{4})")
BYTE_BITS = 8
BYTE = 0xFF
QUERY_STATUS = 0x90
SELECTOR_BIT = 0x01
LOG_LOSS_COUNTERS = ("console_log_dropped_total", "console_log_busy_total",
                     "console_log_truncated_total", "console_log_unavailable_total")


def sent_frames(lines):
    return _frames(lines, TX_LINE)


def collided_frames(lines):
    return _frames(lines, COLLISION_LINE)


def _frames(lines, pattern):
    out = []
    for line in lines:
        match = pattern.search(line)
        if match:
            frame = int(match.group(1), 16)
            out.append((frame >> BYTE_BITS, frame & BYTE))
    return out


def settled_frames(window, quiet_s, max_s, poll_s):
    return settled(lambda: sent_frames(window.lines()), quiet_s, max_s, poll_s)


def violations(lines, fence):
    out, enabled = [], None
    for addr, data in sent_frames(lines):
        try:
            fence.check_frame(addr, data, enabled)
        except LampNotAllowed as exc:
            out.append("%s went out: %s" % (describe_frame(addr, data), exc))
        enabled = enables(addr, data)
    return out


def barrier(short):
    return (short << 1) | SELECTOR_BIT, QUERY_STATUS


def barrier_count(lines, short):
    return sent_frames(lines).count(barrier(short))


def log_losses(stats):
    dali = stats.get("dali") or {}
    return {name: dali.get(name, 0) for name in LOG_LOSS_COUNTERS}


def lost_lines(before, after):
    return {name: after[name] - before[name] for name in before
            if after.get(name, 0) != before.get(name, 0)}
