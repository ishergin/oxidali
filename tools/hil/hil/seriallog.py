import contextlib
import datetime
import os
import re
import time
from dataclasses import dataclass, field
from pathlib import Path

from hil import serialmon

BOOT_MARKER = "HTTP server listening"
LATE_ENTRIES = re.compile(r"DALI ISR late (?:tick|entries)[^:]*: (.*)$")
LATE_ITEM = re.compile(r"(?:^|, )([^,×]+)×([0-9]+) \(max ([0-9]+) us")
ITEM_MARK = "×"
LATE_REPORT_BYTES = 768
NESTED_SUFFIX = "+isr"
HEARTBEAT = re.compile(r"firmware heartbeat: uptime=([0-9]+)s")
HEARTBEAT_PERIOD_S = 60
DUT_STAMP = re.compile(r"\b[EWIDV] \(([0-9]+)\) ")
PHY_LEVEL = re.compile(r"DALI PHY interrupt: level ([0-9]+)")
RESET_BANNER = re.compile(r"ESP-ROM:|rst:0x")
HOST_STAMP = re.compile(r"^([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2})(\.[0-9]+)?Z? ")
HOST_STAMP_FORMAT = "%Y-%m-%dT%H:%M:%S"
MS_PER_S = 1000
BOOT_START_SLACK_S = 5.0
CLOCK_DRIFT = 100e-6
REVERSE_BLOCK_BYTES = 1 << 16


@dataclass
class LateEntries:
    tasks: dict = field(default_factory=dict)
    texts: list = field(default_factory=list)
    unparsed: int = 0
    truncated: int = 0

    def mentioning(self, task):
        return [text for text in self.texts if task in text]


def late_entries(lines):
    found = LateEntries()
    for line in lines:
        match = LATE_ENTRIES.search(line)
        if match:
            _tally(found, match.group(1))
    return found


def _tally(found, text):
    items = LATE_ITEM.findall(text)
    found.texts.append(text)
    found.unparsed += text.count(ITEM_MARK) - len(items)
    found.truncated += len(text.encode()) >= LATE_REPORT_BYTES
    for task, count, gap in items:
        entry = found.tasks.setdefault(task, [0, 0])
        entry[0] += int(count)
        entry[1] = max(entry[1], int(gap))


def task_of(key):
    return key[:-len(NESTED_SUFFIX)] if key.endswith(NESTED_SUFFIX) else key


def heartbeat_run(lines):
    best = run = 0
    last = None
    for match in filter(None, map(HEARTBEAT.search, lines)):
        uptime = int(match.group(1))
        run = run + 1 if last is not None and uptime - last == HEARTBEAT_PERIOD_S else 1
        best, last = max(best, run), uptime
    return best


def dut_ms(line):
    match = DUT_STAMP.search(line)
    return int(match.group(1)) if match else None


def newest_ms(lines):
    return max((stamp for stamp in map(dut_ms, lines) if stamp is not None), default=None)


def logged_past(lines, ms):
    newest = newest_ms(lines)
    return newest is not None and newest >= ms


def host_s(line):
    found = HOST_STAMP.match(line)
    if found is None:
        return None
    whole = datetime.datetime.strptime(found.group(1), HOST_STAMP_FORMAT)
    return (whole.replace(tzinfo=datetime.timezone.utc).timestamp()
            + float(found.group(2) or 0))


def _boot_slack_s(dut_a, dut_b):
    return BOOT_START_SLACK_S + CLOCK_DRIFT * abs(dut_a - dut_b) / MS_PER_S


def running_boot_phy_level(newest_first):
    newest = None
    for line in newest_first:
        if RESET_BANNER.search(line):
            return None
        host, dut = host_s(line), dut_ms(line)
        if host is None or dut is None:
            continue
        start = host - dut / MS_PER_S
        newest = newest or (start, dut)
        if host < newest[0] - _boot_slack_s(newest[1], 0):
            return None
        found = PHY_LEVEL.search(line)
        if found:
            same = abs(start - newest[0]) <= _boot_slack_s(newest[1], dut)
            return int(found.group(1)) if same else None
    return None


def lines_newest_first(path, block=REVERSE_BLOCK_BYTES):
    with open(path, "rb") as fh:
        end, tail = fh.seek(0, os.SEEK_END), b""
        while end > 0:
            start = max(0, end - block)
            fh.seek(start)
            parts = (fh.read(end - start) + tail).split(b"\n")
            tail, end = parts[0], start
            yield from (raw.decode("utf-8", "replace") for raw in reversed(parts[1:]))
        yield tail.decode("utf-8", "replace")


class SerialLog:
    def __init__(self, cfg):
        self.cfg = cfg

    @property
    def monitor_alive(self):
        return serialmon.alive(self.cfg)

    @property
    def log_path(self):
        return serialmon.log_path(self.cfg)

    def window(self):
        return LogWindow(self.log_path)

    def boot_phy_level(self):
        path = Path(self.log_path)
        if not path.exists():
            return None
        with contextlib.closing(lines_newest_first(path)) as lines:
            return running_boot_phy_level(lines)


class LogWindow:
    def __init__(self, path: Path):
        self.path = Path(path)
        self._offset = 0

    def __enter__(self):
        self._offset = self.path.stat().st_size if self.path.exists() else 0
        return self

    def __exit__(self, *exc):
        return False

    def lines(self):
        if not self.path.exists():
            return []
        with open(self.path, errors="replace") as fh:
            fh.seek(self._offset)
            return fh.read().splitlines()

    def expect(self, pattern, timeout_s=10.0):
        rx = re.compile(pattern)
        deadline = time.monotonic() + timeout_s
        while True:
            for line in self.lines():
                if rx.search(line):
                    return line
            if time.monotonic() >= deadline:
                raise AssertionError("serial: no line matching %r within %.0fs"
                                     % (pattern, timeout_s))
            time.sleep(0.5)

    def reboot_detected(self):
        return any(BOOT_MARKER in line for line in self.lines())
