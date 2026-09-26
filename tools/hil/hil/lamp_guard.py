import re
from urllib.parse import unquote

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


INITIALISE, RANDOMISE, PROGRAM_SHORT_ADDRESS = 0xA5, 0xA7, 0xB7
COMMISSIONING_SPECIALS = frozenset({INITIALISE, RANDOMISE, PROGRAM_SHORT_ADDRESS})
GROUP_CONFIG_OPCODES = range(0x60, 0x80)
GROUP_OF_OPCODE = 0x0F
PUT, DELETE = "PUT", "DELETE"
COMMISSION_MODE = "commission_unaddressed"

FENCE_ROUTES = (
    ("group_ts", re.compile(r"adapters/\d+/groups/(\d+)/target-state")),
    ("groups_apply", re.compile(r"adapters/\d+/groups/apply")),
    ("group_meta", re.compile(r"adapters/\d+/groups/(\d+)")),
    ("group_matrix", re.compile(r"adapters/\d+/group-membership-matrix")),
    ("scene_matrix", re.compile(r"adapters/\d+/scenes/(\d+)/matrix")),
    ("scene_apply", re.compile(r"adapters/\d+/scenes/(\d+)/apply")),
    ("scene_recall", re.compile(r"adapters/\d+/scenes/(\d+)/recall")),
    ("scene_meta", re.compile(r"adapters/\d+/scenes/(\d+)")),
    ("hcl", re.compile(r"hcl-schedules(?:/([^/]+))?(?:/override)?")),
    ("rule_run", re.compile(r"rules/([^/]+)/run")),
    ("rules", re.compile(r"rules(?:/([^/]+))?")),
    ("commissioning", re.compile(r"adapters/\d+/commissioning/(?!identify$).+")),
    ("discovery", re.compile(r"adapters/\d+/discovery-runs")),
    ("policies", re.compile(r"policies(?:/apply)?")),
    ("device_config", re.compile(r"adapters/\d+/physical-devices/(\d+)(?:/write-attributes)?")),
    ("vl_binding", re.compile(r"adapters/\d+/virtual-lamps/(\d+)/binding")),
    ("vl", re.compile(r"adapters/\d+/virtual-lamps/(\d+)")),
)


class VirtualFence:
    def __init__(self, park, groups, session_vls, owner_rules="", owner_schedules=(),
                 commissioning=False, pending=None):
        self.park = frozenset(park)
        self.groups = frozenset(groups)
        self.session_vls = frozenset(session_vls)
        self.owner_rules = owner_rules or ""
        self.owner_schedules = frozenset(owner_schedules)
        self.commissioning = bool(commissioning)
        self._pending = pending

    def check_request(self, method, path, body):
        for kind, pattern in FENCE_ROUTES:
            match = pattern.fullmatch(path)
            if match:
                key = match.group(1) if pattern.groups else None
                return getattr(self, "_" + kind)(method, key, body)
        return False

    def refuse(self, what):
        raise LampNotAllowed("%s refused: the virtual-gear session reaches only the park "
                             "SA%s and groups %s" % (what, spell(self.park), spell(self.groups)))

    def _group_ts(self, method, key, body):
        if int(key) not in self.groups:
            self.refuse("target-state of group %s" % key)
        return True

    def _groups_apply(self, method, key, body):
        self._pending_only("group", None)
        return True

    def _group_meta(self, method, key, body):
        allowed = isinstance(body, dict) and set(body) <= {"ha_entity_enabled"} \
            and int(key) in self.groups
        if not allowed:
            self.refuse("%s of group %s %r" % (method, key, body))
        return True

    def _group_matrix(self, method, key, body):
        self._session_rows(method, body, "group membership", self.groups)
        return True

    def _scene_matrix(self, method, key, body):
        self._session_rows(method, body, "scene %s" % key, None)
        return True

    def _scene_apply(self, method, key, body):
        self._pending_only("scene", int(key))
        return True

    def _scene_recall(self, method, key, body):
        scope = body.get("scope") if isinstance(body, dict) else None
        if scope != "group" or body.get("group_id") not in self.groups:
            self.refuse("recall of scene %s with %r" % (key, body))
        return True

    def _scene_meta(self, method, key, body):
        self.refuse("%s of scene %s" % (method, key))

    def _hcl(self, method, key, body):
        if method == DELETE and key in self.owner_schedules:
            self.refuse("deleting the owner's HCL schedule %s" % key)
        for target in (body or {}).get("targets") or [] if isinstance(body, dict) else []:
            if target.get("scope") != "group" or \
                    not set(target.get("group_ids") or ()) <= self.groups:
                self.refuse("HCL target %r" % target)
        return True

    def owns_rule(self, key):
        return unquote(key or "") in RULE_NAME.findall(self.owner_rules)

    def _rule_run(self, method, key, body):
        if self.owns_rule(key):
            self.refuse("running the owner's rule %r" % unquote(key))
        return True

    def _rules(self, method, key, body):
        if method == "POST" and key == RULES_PARSE:
            return True
        if method == "PATCH" and key and not self.owns_rule(key):
            return True
        source = body.get("source") if isinstance(body, dict) else None
        if method != PUT or key or not isinstance(source, str):
            self.refuse("%s rules/%s" % (method, unquote(key or "")))
        new_groups = set(RULE_GROUP.findall(source)) - set(RULE_GROUP.findall(self.owner_rules))
        stray = sorted(int(g) for g in new_groups if int(g) not in self.groups)
        if stray:
            self.refuse("a rules document that drives groups %s" % stray)
        return True

    def _commissioning(self, method, key, body):
        if not self.commissioning:
            self.refuse("commissioning step %s (HIL_ALLOW_VIRTUAL_COMMISSIONING is off)" % key)
        return True

    def _discovery(self, method, key, body):
        mode = body.get("mode") if isinstance(body, dict) else None
        if mode == COMMISSION_MODE and not self.commissioning:
            self.refuse("discovery mode %s" % mode)
        return True

    def _policies(self, method, key, body):
        self.refuse("%s policies" % method)

    def _device_config(self, method, key, body):
        if int(key) not in self.park:
            self.refuse("%s of SA%s" % (method, key))
        return True

    def _vl_binding(self, method, key, body):
        short = body.get("physical_short_address") if isinstance(body, dict) else None
        if int(key) not in self.session_vls or (method == PUT and short not in self.park):
            self.refuse("binding VL%s to %r" % (key, short))
        return True

    def _vl(self, method, key, body):
        if int(key) not in self.session_vls:
            self.refuse("%s of VL%s" % (method, key))
        return True

    def _session_rows(self, method, body, what, groups):
        rows = body.get("rows") if isinstance(body, dict) else body
        if method == PUT or not isinstance(rows, list):
            self.refuse("replacing the whole %s matrix" % what)
        for row in rows:
            if row.get("virtual_lamp_id") not in self.session_vls:
                self.refuse("a %s row of VL%s" % (what, row.get("virtual_lamp_id")))
            desired = row.get("desired") or []
            if groups is not None and any(bit and g not in groups for g, bit in enumerate(desired)):
                self.refuse("VL%s into groups outside %s" % (row.get("virtual_lamp_id"),
                                                             spell(groups)))

    def _pending_only(self, kind, scene):
        pending = set(self._pending(kind, scene)) if self._pending else None
        if pending is None or not pending <= self.session_vls:
            self.refuse("an apply whose diff reaches VL%s" % spell((pending or set())
                                                                  - self.session_vls))

    def check_frame(self, addr, data):
        if addr in COMMISSIONING_SPECIALS and not self.commissioning:
            raise LampNotAllowed("commissioning frame 0x%02X refused: "
                                 "HIL_ALLOW_VIRTUAL_COMMISSIONING is off" % addr)
        target = wire_target(addr)
        if target is None or not frame_writes(addr, data):
            return False
        if addr >= BROADCAST_FIRST:
            raise LampNotAllowed("%s refused: broadcast never runs on a shared wire"
                                 % describe_frame(addr, data))
        if target == TARGET_SEGMENT:
            if (addr >> 1) & GROUP_MASK not in self.groups:
                raise LampNotAllowed("%s refused: the group is outside %s"
                                     % (describe_frame(addr, data), spell(self.groups)))
            return True
        if addr & 1 and data in GROUP_CONFIG_OPCODES and data & GROUP_OF_OPCODE not in self.groups:
            raise LampNotAllowed("%s refused: it joins or leaves a group outside %s"
                                 % (describe_frame(addr, data), spell(self.groups)))
        return False


RULE_GROUP = re.compile(r"\bgroup\((\d+)\)")
RULE_NAME = re.compile(r'^\s*rule\s+"([^"]*)"', re.MULTILINE)
RULES_PARSE = "parse"


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
        self.fence = None

    @classmethod
    def for_config(cls, cfg, segment=None, binding=None):
        return cls(cfg.lamp_short_set(), cfg.lamps_read_only, segment, binding)

    def check_request(self, method, path, body=None):
        if method.upper() in READ_METHODS:
            return
        path = path.lstrip("/").split("?", 1)[0]
        if self.fence is not None and self.fence.check_request(method.upper(), path, body):
            return
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
        if self.fence is not None and self.fence.check_frame(addr, data):
            return
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
