import re
import time
from pathlib import Path

from hil import serialmon

BOOT_MARKER = "HTTP server listening"
LATE_ENTRIES = re.compile(r"DALI ISR late (?:tick|entries)[^:]*: (.*)$")
LATE_ITEM = re.compile(r"([\w\-+.]+)×(\d+) \(max (\d+) us")
NESTED_SUFFIX = "+isr"


def late_entry_tasks(lines):
    tally = {}
    for line in lines:
        match = LATE_ENTRIES.search(line)
        if not match:
            continue
        for task, count, gap in LATE_ITEM.findall(match.group(1)):
            entry = tally.setdefault(task, [0, 0])
            entry[0] += int(count)
            entry[1] = max(entry[1], int(gap))
    return tally


def task_of(key):
    return key[:-len(NESTED_SUFFIX)] if key.endswith(NESTED_SUFFIX) else key


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
