import re
import time
from dataclasses import dataclass

from hil import remote_serial, role, serialmon
from hil.lamp_guard import spell
from hil.seriallog import LogWindow

REPLY_QUIET_S = 0.4
REPLY_TIMEOUT_S = 5.0
POLL_S = 0.1

STAMP = re.compile(r"^\S+Z ")
_KV = re.compile(r"(\w+)=(-?\d+)")
RESERVED = re.compile(r"^(?:refused: )?reserved (?:already )?(0x[0-9a-f]+)")
SHOW_FIELDS = 8
UNTOUCHED_QUIET_S = 1.0
UNTOUCHED_MAX_S = 5.0
ARROW = "->"


class GearSimUnavailable(RuntimeError):
    pass


def strip_stamp(line: str) -> str:
    return STAMP.sub("", line.rstrip("\r\n"), count=1)


@dataclass(frozen=True)
class Change:
    at_us: int
    who: str
    field: str
    old: str
    new: str

    @property
    def short(self):
        return int(self.who[1:]) if self.who.startswith("A") else None


def parse_change(line: str):
    tokens = strip_stamp(line).split()
    if len(tokens) < 6 or tokens[0] != "C" or ARROW not in tokens:
        return None
    arrow = tokens.index(ARROW)
    if arrow < 5 or arrow + 1 >= len(tokens):
        return None
    try:
        at_us = int(tokens[1])
    except ValueError:
        return None
    return Change(at_us, tokens[2], " ".join(tokens[3:arrow - 1]), tokens[arrow - 1],
                  tokens[arrow + 1])


class GearSim:
    def __init__(self, peer_cfg):
        if not role.is_gear_sim(peer_cfg):
            raise GearSimUnavailable("the peer does not run the gear emulator "
                                     "(`hil --peer role gear-sim`)")
        health = role.controller_health(peer_cfg)
        if health is not None:
            raise GearSimUnavailable("the peer answers HTTP as a %s controller: a reset handed "
                                     "the OTA role back" % health.get("role"))
        if not serialmon.alive(peer_cfg):
            raise GearSimUnavailable("the peer's serial monitor is not running")
        self.cfg = peer_cfg
        self.log = serialmon.log_path(peer_cfg)

    def close(self):
        pass

    def window(self) -> LogWindow:
        return LogWindow(self.log)

    def command(self, verb, timeout_s=REPLY_TIMEOUT_S):
        with self.window() as window:
            remote_serial.control(self.cfg, "write " + verb)
            return _replies(window, timeout_s)

    def reserve(self, shorts):
        replies = self.command("reserve " + spell(shorts))
        mask = sum(1 << s for s in set(shorts))
        for line in replies:
            echo = RESERVED.match(line)
            if echo and int(echo.group(1), 16) == mask:
                return line
        raise GearSimUnavailable("the emulator holds another reserve than %s (it takes one "
                                 "per boot): %s" % (spell(shorts), " | ".join(replies)
                                                    or "no reply"))

    def fleet(self, base, dt6, cct, rgb):
        return _expect(self.command("fleet %d %d %d %d" % (base, dt6, cct, rgb)), "fleet ")

    def enable(self, which="all"):
        return _expect(self.command("enable %s" % which), "enabled ")

    def disable(self, which="all"):
        return _expect(self.command("disable %s" % which), "disabled ")

    def log_level(self, level):
        return _expect(self.command("log %s" % level), "log level ")

    def show(self):
        rows = []
        for line in self.command("show"):
            fields = line.split()
            if len(fields) == SHOW_FIELDS and fields[0].isdigit() and fields[1].isdigit():
                rows.append({"short": int(fields[1]), "random": int(fields[2], 16),
                             "dt8": fields[3] == "DT8", "level": int(fields[4]),
                             "groups": int(fields[5], 16), "enabled": fields[6] == "yes"})
        return rows

    def stats(self):
        values = {}
        for line in self.command("stats"):
            for key, number in _KV.findall(line):
                values[key] = int(number)
        return values

    def settle_bands(self):
        bands = {}
        for line in self.command("stats"):
            match = re.match(r"priority (\d) (\d+)", line)
            if match:
                bands[int(match.group(1))] = int(match.group(2))
        return bands

    def violations(self):
        stats = self.stats()
        return {
            key: stats.get(key, 0)
            for key in ("enable_consumed", "below_p1_floor", "send_twice_interloper")
        }


def _replies(window, timeout_s):
    deadline = time.monotonic() + timeout_s
    replies, changed_at = [], time.monotonic()
    while time.monotonic() < deadline:
        now = [strip_stamp(line) for line in window.lines()]
        current = [line[1:].strip() for line in now if line.startswith("#")]
        if len(current) != len(replies):
            replies, changed_at = current, time.monotonic()
        elif replies and time.monotonic() - changed_at > REPLY_QUIET_S:
            break
        time.sleep(POLL_S)
    return replies


def _expect(replies, prefix):
    for line in replies:
        if line.startswith(prefix):
            return line
    raise GearSimUnavailable("the emulator did not confirm (%s…): %s"
                             % (prefix.strip(), " | ".join(replies) or "no reply"))


class GearOracle:
    def __init__(self, sim: GearSim):
        self.sim = sim

    def window(self) -> LogWindow:
        return self.sim.window()

    def changes(self, window):
        return [c for c in (parse_change(line) for line in window.lines()) if c]

    def expect(self, window, short, field, new, timeout_s=REPLY_TIMEOUT_S) -> Change:
        deadline = time.monotonic() + timeout_s
        while True:
            for change in self.changes(window):
                if change.short == short and change.field == field and change.new == str(new):
                    return change
            if time.monotonic() >= deadline:
                seen = [c for c in self.changes(window) if c.short == short]
                raise AssertionError("gear SA%d never reported %s -> %s within %.0fs; its "
                                     "changes: %s" % (short, field, new, timeout_s, seen))
            time.sleep(POLL_S)

    def untouched(self, window, shorts):
        self._await_quiet(window)
        moved = [c for c in self.changes(window) if c.short in set(shorts)]
        assert not moved, "gear that must stay still changed: %s" % moved

    def _await_quiet(self, window):
        deadline = time.monotonic() + UNTOUCHED_MAX_S
        count, still_since = -1, time.monotonic()
        while time.monotonic() < deadline:
            now = len(self.changes(window))
            if now != count:
                count, still_since = now, time.monotonic()
            elif time.monotonic() - still_since >= UNTOUCHED_QUIET_S:
                return
            time.sleep(POLL_S)
