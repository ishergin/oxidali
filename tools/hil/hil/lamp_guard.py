import re

TARGET_SEGMENT = -1
SHORT_LIMIT = 0x80
GROUP_LAST = 0x9F
GROUP_MASK = 0x0F
BROADCAST_FIRST = 0xFC
BYTE = 0xFF
FORWARD16_LIMIT = 0x10000
FIRST_QUERY_OPCODE = 0x90
ARC_POWER_OPCODES = range(0x00, 0x20)
RESET_OPCODE = 0x20
IDENTIFY_DEVICE_OPCODE = 0x25
ACTIVATE_OPCODE = 0xE2
VISIBLE_OPCODES = frozenset(ARC_POWER_OPCODES) | {RESET_OPCODE, IDENTIFY_DEVICE_OPCODE,
                                                  ACTIVATE_OPCODE}
READ_METHODS = ("GET", "HEAD")
DIAGNOSTIC_PREFIX = "dali/"

SHORT, SEGMENT, LAMP, IDENTIFY = "short", "segment", "lamp", "identify"
RESOURCES = (
    (re.compile(r"adapters/\d+/physical-devices/(\d+)/target-state"), SHORT,
     "target-state of SA%s"),
    (re.compile(r"adapters/\d+/groups/(\d+)/target-state"), SEGMENT,
     "target-state of group %s"),
    (re.compile(r"adapters/\d+/virtual-lamps/(\d+)/target-state"), LAMP,
     "target-state of VL%s"),
    (re.compile(r"adapters/\d+/scenes/(\d+)/recall"), SEGMENT, "recall of scene %s"),
    (re.compile(r"adapters/\d+/commissioning/identify"), IDENTIFY, "IDENTIFY DEVICE of SA%s"),
)


class LampNotAllowed(RuntimeError):
    pass


def wire_target(addr):
    if not isinstance(addr, int) or isinstance(addr, bool) or addr < 0:
        return None
    if addr < SHORT_LIMIT:
        return addr >> 1
    if addr <= GROUP_LAST or addr >= BROADCAST_FIRST:
        return TARGET_SEGMENT
    return None


def _is_byte(value):
    return isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= BYTE


def diagnostic_frame(path, body):
    if not isinstance(body, dict):
        return None
    addr = body.get("wire_address")
    if path == "dali/level" and _is_byte(addr):
        return addr & ~1, body.get("level")
    if path == "dali/command" and _is_byte(addr) and _is_byte(body.get("command")):
        return addr, body["command"]
    frame = body.get("frame")
    if path == "dali/raw" and isinstance(frame, int) and 0 <= frame < FORWARD16_LIMIT:
        return frame >> 8, frame & BYTE
    return None


def frame_writes(addr, data):
    if wire_target(addr) is None:
        return False
    return addr & 1 == 0 or not isinstance(data, int) or data < FIRST_QUERY_OPCODE \
        or data == ACTIVATE_OPCODE


def frame_visible(addr, data):
    return wire_target(addr) is not None and (addr & 1 == 0 or data in VISIBLE_OPCODES)


def describe_frame(addr, data):
    verb = "DAPC" if addr & 1 == 0 else "command 0x%02X" % data
    if addr < SHORT_LIMIT:
        scope = "SA%d" % (addr >> 1)
    elif addr <= GROUP_LAST:
        scope = "group %d" % ((addr >> 1) & GROUP_MASK)
    else:
        scope = "broadcast"
    return "%s to wire address 0x%02X (%s)" % (verb, addr, scope)


def spell(shorts):
    return ",".join(str(s) for s in sorted(shorts)) or "(none)"


def named(shorts):
    return ", ".join("SA%d" % s for s in sorted(shorts))


class LampGuard:
    def __init__(self, allowed, read_only=False, segment=None, binding=None):
        self.allowed = frozenset(allowed)
        self.read_only = bool(read_only)
        self._segment = segment
        self._binding = binding

    @classmethod
    def for_config(cls, cfg, segment=None, binding=None):
        return cls(cfg.lamp_short_set(), cfg.lamps_read_only, segment, binding)

    def check_request(self, method, path, body=None):
        if method.upper() in READ_METHODS:
            return
        path = path.lstrip("/").split("?", 1)[0]
        if path.startswith(DIAGNOSTIC_PREFIX):
            frame = diagnostic_frame(path, body)
            if frame is None:
                raise LampNotAllowed(
                    "POST %s %r refused: the lamp guard cannot tell which gear it "
                    "reaches" % (path, body))
            self.check_frame(*frame)
            return
        for pattern, kind, label in RESOURCES:
            match = pattern.fullmatch(path)
            if match:
                key = match.group(1) if pattern.groups else None
                target = self._resource_target(kind, key, body)
                if target is not None:
                    self.check_target(target, True, label % (target if key is None else key))
                return

    def _resource_target(self, kind, key, body):
        if kind == SHORT:
            return int(key)
        if kind == SEGMENT:
            return TARGET_SEGMENT
        if kind == LAMP:
            return None if self._binding is None else self._binding(int(key))
        short = body.get("short_address") if isinstance(body, dict) else None
        return short if isinstance(short, int) else None

    def check_frame(self, addr, data):
        target = wire_target(addr)
        if target is None or not frame_writes(addr, data):
            return
        self.check_target(target, frame_visible(addr, data), describe_frame(addr, data))

    def check_target(self, target, visible, what):
        if visible and self.read_only:
            raise LampNotAllowed("%s refused: HIL_LAMPS_READ_ONLY=1 forbids every visible "
                                 "action" % what)
        if target != TARGET_SEGMENT:
            if target not in self.allowed:
                raise LampNotAllowed("%s refused: SA%d is outside HIL_LAMP_SHORTS=%s"
                                     % (what, target, spell(self.allowed)))
            return
        if self._segment is None:
            raise LampNotAllowed("%s refused: it reaches the whole segment, and no "
                                 "controller lists the gear on it" % what)
        outside = set(self._segment()) - self.allowed
        if outside:
            raise LampNotAllowed("%s refused: it reaches the whole segment, and %s "
                                 "%s outside HIL_LAMP_SHORTS=%s"
                                 % (what, named(outside),
                                    "is" if len(outside) == 1 else "are",
                                    spell(self.allowed)))
