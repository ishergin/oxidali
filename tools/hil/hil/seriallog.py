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
STAMP_REORDER_SLACK_MS = 1000


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


def last_boot_phy_level(lines):
    level = last = None
    for line in lines:
        found = PHY_LEVEL.search(line)
        stamp = dut_ms(line)
        if found:
            level = int(found.group(1))
        elif _a_later_boot(line, stamp, last):
            level = None
        if stamp is not None:
            last = stamp
    return level


def _a_later_boot(line, stamp, last):
    if RESET_BANNER.search(line):
        return True
    return stamp is not None and last is not None and stamp + STAMP_REORDER_SLACK_MS < last


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
        with open(path, errors="replace") as fh:
            return last_boot_phy_level(fh)


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
