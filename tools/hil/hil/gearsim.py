import re
import time
from dataclasses import dataclass

from hil import remote_serial, role, serialmon
from hil.lamp_guard import describe_frame, spell
from hil.seriallog import LogWindow
from hil.wait import settled

REPLY_QUIET_S = 0.4
REPLY_TIMEOUT_S = 5.0
POLL_S = 0.1

STAMP = re.compile(r"^\S+Z ")
_KV = re.compile(r"(\w+)=(-?\d+)")
RESERVED = re.compile(r"^(?:refused: )?reserved (?:already )?(0x[0-9a-f]+)")
LOG_LEVEL_NOW = re.compile(r"\(now: (\w+)\)")
SHOW_FIELDS = 8
UNTOUCHED_QUIET_S = 1.0
UNTOUCHED_MAX_S = 5.0
ARROW = "->"
FRAME_TOKENS = 4
HEX = 16
FRAME_LEVEL = "frame"
HEARING_COUNTERS = ("log_dropped", "decode_failed", "ring_dropped")

SELECTOR = 0x01
GROUP_BASE = 0x80
BROADCAST_ARC = 0xFE
MASK = 0xFF
ARC_POWER_COMMANDS = range(0x00, 0x20)
QUERY_STATUS = 0x90
ENABLE_DEVICE_TYPE_8 = (0xC1, 0x08)
SET_TEMPORARY_COLOUR_TEMPERATURE = 0xE7
ACTIVATE = 0xE2
DT8_WRITES = (SET_TEMPORARY_COLOUR_TEMPERATURE, ACTIVATE)
DTR0, DTR1 = 0xA3, 0xC3
BYTE_BITS = 8


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


@dataclass(frozen=True)
class Heard:
    at_us: int
    address: int
    data: int


def parse_heard(line: str):
    tokens = strip_stamp(line).split()
    if len(tokens) < FRAME_TOKENS or tokens[0] != "F":
        return None
    try:
        return Heard(int(tokens[1]), int(tokens[2], HEX), int(tokens[3], HEX))
    except ValueError:
        return None


def heard_frames(lines):
    return [frame for frame in map(parse_heard, lines) if frame]


def unheard(sent, heard):
    pairs, missed, at = [(frame.address, frame.data) for frame in heard], [], 0
    for frame in sent:
        try:
            at = pairs.index(frame, at) + 1
        except ValueError:
            missed.append(frame)
    return missed


def moved(before, after, names):
    return {name: after.get(name, 0) - before.get(name, 0) for name in names
            if after.get(name, 0) != before.get(name, 0)}


def command_address(short):
    return (short << 1) | SELECTOR


def group_dapc_address(group):
    return GROUP_BASE | (group << 1)


@dataclass
class ColourWrites:
    units: list
    gates: list
    faults: list

    def activated(self):
        out = []
        for value, done in self.units:
            if done and (not out or out[-1] != value):
                out.append(value)
        return out


def colour_writes(frames, short):
    tally, dtr, previous = ColourWrites([], [], []), {}, None
    for frame in frames:
        if frame.address in (DTR0, DTR1):
            dtr[frame.address] = frame.data
        elif frame.address == command_address(short):
            _colour_step(tally, frame, previous == ENABLE_DEVICE_TYPE_8, dtr, short)
        previous = (frame.address, frame.data)
    if tally.units and not tally.units[-1][1]:
        tally.faults.append("%s mirek staged on SA%d was never activated"
                            % (tally.units[-1][0], short))
    return tally


def _colour_step(tally, frame, enabled, dtr, short):
    if frame.data in DT8_WRITES and not enabled:
        tally.faults.append("0x%02X to SA%d at %d us did not follow ENABLE DEVICE TYPE 8"
                            % (frame.data, short, frame.at_us))
    pending = tally.units and not tally.units[-1][1]
    if frame.data == SET_TEMPORARY_COLOUR_TEMPERATURE:
        _stage(tally, staged_value(dtr), frame, short)
    elif frame.data == QUERY_STATUS and pending:
        tally.gates.append("QUERY STATUS to SA%d at %d us sat between a staged colour and "
                           "its ACTIVATE" % (short, frame.at_us))
    elif frame.data == ACTIVATE and tally.units:
        tally.units[-1][1] = True


def _stage(tally, value, frame, short):
    if tally.units and not tally.units[-1][1]:
        if tally.units[-1][0] == value:
            return
        tally.faults.append("%s mirek staged on SA%d was replaced by %s at %d us before an "
                            "ACTIVATE" % (tally.units[-1][0], short, value, frame.at_us))
    tally.units.append([value, False])


def staged_value(dtr):
    if DTR0 not in dtr or DTR1 not in dtr:
        return None
    return dtr[DTR1] << BYTE_BITS | dtr[DTR0]


def mask_frames(frames, group):
    return sum(1 for frame in frames
               if (frame.address, frame.data) == (group_dapc_address(group), MASK))


def level_moves(frames, group, members):
    target = (group_dapc_address(group), MASK)
    first = next((i for i, frame in enumerate(frames)
                  if (frame.address, frame.data) == target), None)
    after = frames[first + 1:] if first is not None else []
    return ["%s at %d us moved the level after the stop"
            % (describe_frame(frame.address, frame.data), frame.at_us)
            for frame in after if moves_level(frame, group, members)]


def moves_level(frame, group, members):
    targets = {group_dapc_address(group), BROADCAST_ARC} | {short << 1 for short in members}
    if frame.address & ~SELECTOR not in targets:
        return False
    if frame.address & SELECTOR:
        return frame.data in ARC_POWER_COMMANDS
    return frame.data != MASK


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

    def log_level_now(self):
        for line in self.command("log"):
            match = LOG_LEVEL_NOW.search(line)
            if match:
                return match.group(1)
        raise GearSimUnavailable("the emulator did not name its log level")

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


class HeardWindow(LogWindow):
    def __init__(self, sim):
        super().__init__(sim.log)
        self.sim, self.level, self.before = sim, None, {}

    def __enter__(self):
        self.level = self.sim.log_level_now()
        self.before = self.sim.stats()
        super().__enter__()
        self.sim.log_level(FRAME_LEVEL)
        return self

    def __exit__(self, *exc):
        self.sim.log_level(self.level)
        return False

    def heard(self):
        return heard_frames(self.lines())

    def heard_settled(self, quiet_s, max_s, poll_s):
        return settled(self.heard, quiet_s, max_s, poll_s)

    def losses(self):
        return moved(self.before, self.sim.stats(), HEARING_COUNTERS)


class GearOracle:
    def __init__(self, sim: GearSim):
        self.sim = sim

    def window(self) -> LogWindow:
        return self.sim.window()

    def hearing(self) -> HeardWindow:
        return HeardWindow(self.sim)

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
        settled(lambda: self.changes(window), UNTOUCHED_QUIET_S, UNTOUCHED_MAX_S, POLL_S)
