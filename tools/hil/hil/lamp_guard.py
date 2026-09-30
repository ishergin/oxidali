import re
from dataclasses import dataclass
from urllib.parse import unquote

from hil.write_log import ALL, EVERY, frame_keys, request_keys, restarts

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
SET_MAX_LEVEL, SET_MIN_LEVEL = 0x2A, 0x2B
VISIBLE_OPCODES = frozenset(ARC_POWER_OPCODES) | {RESET_OPCODE, IDENTIFY_DEVICE_OPCODE,
                                                  SET_MAX_LEVEL, SET_MIN_LEVEL}
EXTENDED_FIRST = 0xE0
QUERY_EXTENDED_VERSION = 0xFF
DT6, DT8 = 6, 8
EXTENDED_QUERY_FIRST = {DT6: 0xED, DT8: 0xF7}
DT6_CONFIG_END = 0xE5
DT6_OPCODES = (frozenset(range(EXTENDED_FIRST, DT6_CONFIG_END))
               | frozenset(range(EXTENDED_QUERY_FIRST[DT6], BYTE + 1)))
REFERENCE_SYSTEM_POWER, SELECT_DIMMING_CURVE = 0xE0, 0xE3
ACTIVATE_OPCODE = 0xE2
XY_STEPS = range(0xE3, 0xE7)
TC_STEPS = range(0xE8, 0xEA)
STORE_TC_LIMIT, START_AUTO_CALIBRATION = 0xF2, 0xF6
VISIBLE_EXTENDED = {
    DT6: frozenset({REFERENCE_SYSTEM_POWER, SELECT_DIMMING_CURVE}),
    DT8: (frozenset(XY_STEPS) | frozenset(TC_STEPS)
          | {ACTIVATE_OPCODE, STORE_TC_LIMIT, START_AUTO_CALIBRATION}),
}
VISIBLE_ATTRIBUTES = frozenset({"max_level", "min_level", "dimming_curve",
                                "tc_coolest_mirek", "tc_warmest_mirek"})
UNSEEN = "unseen"
READ_METHODS = ("GET", "HEAD")
DIAGNOSTIC_PREFIX = "dali/"

SHORT, SEGMENT, LAMP, IDENTIFY = "short", "segment", "lamp", "identify"
VISIBLE = True


def attribute_write_visible(body):
    return not isinstance(body, dict) or bool(VISIBLE_ATTRIBUTES & set(body))


RESOURCES = (
    (re.compile(r"adapters/[0-9]+/physical-devices/([0-9]+)/target-state"), SHORT,
     "target-state of SA%s", VISIBLE),
    (re.compile(r"adapters/[0-9]+/groups/([0-9]+)/target-state"), SEGMENT,
     "target-state of group %s", VISIBLE),
    (re.compile(r"adapters/[0-9]+/virtual-lamps/([0-9]+)/target-state"), LAMP,
     "target-state of VL%s", VISIBLE),
    (re.compile(r"adapters/[0-9]+/scenes/([0-9]+)/recall"), SEGMENT, "recall of scene %s",
     VISIBLE),
    (re.compile(r"adapters/[0-9]+/commissioning/identify"), IDENTIFY,
     "IDENTIFY DEVICE of SA%s", VISIBLE),
    (re.compile(r"adapters/[0-9]+/physical-devices/([0-9]+)/write-attributes"), SHORT,
     "attribute write to SA%s", attribute_write_visible),
)


class LampNotAllowed(RuntimeError):
    pass


TERMINATE, DTR0, INITIALISE, RANDOMISE, COMPARE, WITHDRAW, PING = (
    0xA1, 0xA3, 0xA5, 0xA7, 0xA9, 0xAB, 0xAD)
SEARCH_ADDRESS_H, SEARCH_ADDRESS_M, SEARCH_ADDRESS_L = 0xB1, 0xB3, 0xB5
PROGRAM_SHORT_ADDRESS, VERIFY_SHORT_ADDRESS, QUERY_SHORT_ADDRESS = 0xB7, 0xB9, 0xBB
ENABLE_DEVICE_TYPE, DTR1, DTR2 = 0xC1, 0xC3, 0xC5
SPECIAL_FIRST = 0xA0
INITIALISE_UNADDRESSED = 0xFF
SETUP_SPECIALS = frozenset({TERMINATE, DTR0, PING, ENABLE_DEVICE_TYPE, DTR1, DTR2})
COMMISSIONING_SPECIALS = frozenset({INITIALISE, RANDOMISE, COMPARE, WITHDRAW,
                                    SEARCH_ADDRESS_H, SEARCH_ADDRESS_M, SEARCH_ADDRESS_L,
                                    PROGRAM_SHORT_ADDRESS, VERIFY_SHORT_ADDRESS,
                                    QUERY_SHORT_ADDRESS})
PARK_OPERAND_SPECIALS = frozenset({PROGRAM_SHORT_ADDRESS, VERIFY_SHORT_ADDRESS})
COMMISSIONING_REFUSAL = ("commissioning never runs on the installation: only the virtual "
                         "tier's expert steps reach emulated gear, with "
                         "HIL_ALLOW_VIRTUAL_COMMISSIONING=1")
COMMISSIONING_ROUTES = (
    re.compile(r"adapters/[0-9]+/commissioning/(steps/[a-z-]+|address-changes|replacements)"),
    re.compile(r"adapters/[0-9]+/input-devices/commission"),
)
DISCOVERY_ROUTE = re.compile(r"adapters/[0-9]+/discovery-runs")
APPLY_ROUTES = (("group", re.compile(r"adapters/[0-9]+/groups/apply")),
                ("scene", re.compile(r"adapters/[0-9]+/scenes/([0-9]+)/apply")))
SCAN_MODES = frozenset({"scan_known_short_addresses", "refresh_known"})
GROUP_CONFIG_OPCODES = range(0x60, 0x80)
GROUP_OF_OPCODE = 0x0F
PUT, PATCH, POST = "PUT", "PATCH", "POST"
UNADDRESSED_SCOPE = "unaddressed"
STEPS_ANY_OPERAND = frozenset({"initialise", "randomise", "search-address", "compare",
                               "withdraw", "terminate"})
STEPS_PARK_OPERAND = frozenset({"program-short-address", "verify-short-address"})

VIRTUAL_ROUTES = (
    ("lamp_ts", re.compile(r"adapters/\d+/virtual-lamps/(\d+)/target-state")),
    ("device_ts", re.compile(r"adapters/\d+/physical-devices/(\d+)/target-state")),
    ("group_ts", re.compile(r"adapters/\d+/groups/(\d+)/target-state")),
    ("group_matrix", re.compile(r"adapters/\d+/group-membership-matrix")),
    ("groups_apply", re.compile(r"adapters/\d+/groups/apply")),
    ("scene_matrix", re.compile(r"adapters/\d+/scenes/(\d+)/matrix")),
    ("scene_apply", re.compile(r"adapters/\d+/scenes/(\d+)/apply")),
    ("identify", re.compile(r"adapters/\d+/commissioning/identify")),
    ("step", re.compile(r"adapters/\d+/commissioning/steps/([a-z-]+)")),
    ("attr_read", re.compile(r"adapters/\d+/physical-devices/(\d+)/attribute-reads")),
    ("attr_write", re.compile(r"adapters/\d+/physical-devices/(\d+)/write-attributes")),
    ("rules", re.compile(r"rules")),
    ("rule_toggle", re.compile(r"rules/([^/]+)")),
    ("rule_run", re.compile(r"rules/([^/]+)/run")),
)

RULE_SEPARATOR = "\n\n"
TEST_RULE_PREFIX = "hil-"
GROUP_TARGET = "group"
HTTP_RULE = 'rule "%s" {\n  when http trigger\n  do   %s(%d).%s\n}'
TEST_RULE = re.compile(
    r'rule "(hil-[a-z0-9-]+)" \{\n'
    r'  when http trigger\n'
    r'  do   (group|lamp)\(([0-9]+)\)\.[a-z_]+\([^()\n]*\)\n'
    r'\}')


HIL_RULE = re.compile(r'rule "hil-[^"\n]*" \{[^{}]*\}')


def http_rule(name, target, key, action):
    return HTTP_RULE % (name, target, key, action)


def appended_blocks(baseline, source):
    if source == baseline:
        return []
    prefix = baseline + RULE_SEPARATOR if baseline else ""
    if not source.startswith(prefix):
        return None
    return source[len(prefix):].split(RULE_SEPARATOR)


def only_hil_rules_appended(baseline, source):
    blocks = appended_blocks(baseline, source)
    return blocks is not None and all(HIL_RULE.fullmatch(block) for block in blocks)


@dataclass(frozen=True)
class RulesBaseline:
    source: str
    toggles: dict


def appended_test_rules(baseline, source):
    blocks = appended_blocks(baseline, source)
    if blocks is None:
        return None
    found = []
    for block in blocks:
        match = TEST_RULE.fullmatch(block)
        if match is None:
            return None
        found.append((match.group(1), match.group(2), int(match.group(3))))
    return found


class VirtualFence:
    def __init__(self, park, groups, session_vls, commissioning=False, pending=None,
                 rules=None):
        self.park = frozenset(park)
        self.groups = frozenset(groups)
        self.session_vls = frozenset(session_vls)
        self.commissioning = bool(commissioning)
        self._pending = pending
        self.rules = rules
        self.test_rules = set()

    def check_request(self, method, path, body):
        if path.startswith(DIAGNOSTIC_PREFIX):
            return False
        for kind, pattern in VIRTUAL_ROUTES:
            match = pattern.fullmatch(path)
            if match:
                key = match.group(1) if pattern.groups else None
                getattr(self, "_" + kind)(method, key, body)
                return True
        self.refuse("%s %s (not on the virtual tier's list)" % (method, path))

    def refuse(self, what):
        raise LampNotAllowed("%s refused: the virtual-gear session reaches only the park "
                             "SA%s and groups %s" % (what, spell(self.park), spell(self.groups)))

    def _lamp_ts(self, method, key, body):
        if method != PUT or int(key) not in self.session_vls:
            self.refuse("%s target-state of VL%s" % (method, key))

    def _device_ts(self, method, key, body):
        if method != PUT or int(key) not in self.park:
            self.refuse("%s target-state of SA%s" % (method, key))

    def _group_ts(self, method, key, body):
        if method != PUT or int(key) not in self.groups:
            self.refuse("%s target-state of group %s" % (method, key))

    def _group_matrix(self, method, key, body):
        self._session_rows(method, body, "group membership", self.groups)

    def _groups_apply(self, method, key, body):
        self._pending_only(method, "group", None)

    def _scene_matrix(self, method, key, body):
        self._session_rows(method, body, "scene %s" % key, None)

    def _scene_apply(self, method, key, body):
        self._pending_only(method, "scene", int(key))

    def _identify(self, method, key, body):
        short = body.get("short_address") if isinstance(body, dict) else None
        if method != POST or short not in self.park:
            self.refuse("IDENTIFY DEVICE of %r" % short)

    def _step(self, method, key, body):
        if not self.commissioning:
            self.refuse("commissioning step %s (HIL_ALLOW_VIRTUAL_COMMISSIONING is off)" % key)
        body = body if isinstance(body, dict) else {}
        if method != POST or key not in STEPS_ANY_OPERAND | STEPS_PARK_OPERAND:
            self.refuse("%s commissioning step %s" % (method, key))
        if key == "initialise" and body.get("scope") != UNADDRESSED_SCOPE:
            self.refuse("initialise with %r: only gear without a short address may enter "
                        "initialisation" % body)
        if key in STEPS_PARK_OPERAND and body.get("short_address") not in self.park:
            self.refuse("%s to %r, outside the park" % (key, body.get("short_address")))

    def _attr_read(self, method, key, body):
        self._park_post(method, key, "attribute read")

    def _attr_write(self, method, key, body):
        self._park_post(method, key, "attribute write")

    def _park_post(self, method, key, what):
        if method != POST or int(key) not in self.park:
            self.refuse("%s %s of SA%s" % (method, what, key))

    def _rules(self, method, key, body):
        source = body.get("source") if isinstance(body, dict) else None
        added = None
        if method == PUT and self.rules is not None and isinstance(source, str):
            added = appended_test_rules(self.rules.source, source)
        if added is None:
            self.refuse("%s of a rules document other than the owner's plus HTTP-triggered "
                        "test rules" % method)
        for name, kind, target in added:
            if target not in (self.groups if kind == GROUP_TARGET else self.session_vls):
                self.refuse("test rule %r on %s(%d)" % (name, kind, target))
        self.test_rules.update(name for name, _, _ in added)

    def _rule_toggle(self, method, key, body):
        name = unquote(key)
        if method == PATCH and name.startswith(TEST_RULE_PREFIX) and body == {"enabled": False}:
            return
        owner = self.rules.toggles if self.rules is not None else {}
        if method != PATCH or name not in owner or body != {"enabled": owner[name]}:
            self.refuse("%s of rule %r other than its toggle back to %r"
                        % (method, name, owner.get(name)))

    def _rule_run(self, method, key, body):
        name = unquote(key)
        if method != POST or name not in self.test_rules:
            self.refuse("%s run of rule %r, which no test appended" % (method, name))

    def _session_rows(self, method, body, what, groups):
        rows = body.get("rows") if isinstance(body, dict) else body
        if method != PATCH or not isinstance(rows, list):
            self.refuse("%s of the whole %s matrix" % (method, what))
        for row in rows:
            if row.get("virtual_lamp_id") not in self.session_vls:
                self.refuse("a %s row of VL%s" % (what, row.get("virtual_lamp_id")))
            desired = row.get("desired") or []
            if groups is not None and any(bit and g not in groups for g, bit in enumerate(desired)):
                self.refuse("VL%s into groups outside %s" % (row.get("virtual_lamp_id"),
                                                             spell(groups)))

    def _pending_only(self, method, kind, scene):
        pending = set(self._pending(kind, scene)) if self._pending else None
        if method != POST or pending is None or not pending <= self.session_vls:
            self.refuse("an apply whose diff reaches VL%s" % spell((pending or set())
                                                                  - self.session_vls))

    def check_frame(self, addr, data, enabled=None):
        if SPECIAL_FIRST <= addr < BROADCAST_FIRST:
            self._special(addr, data)
            return True
        target = wire_target(addr)
        if target is None or not frame_writes(addr, data, enabled):
            return False
        if addr >= BROADCAST_FIRST:
            raise LampNotAllowed("%s refused: broadcast never runs on a shared wire"
                                 % describe_frame(addr, data))
        if target == TARGET_SEGMENT and (addr >> 1) & GROUP_MASK not in self.groups:
            raise LampNotAllowed("%s refused: the group is outside %s"
                                 % (describe_frame(addr, data), spell(self.groups)))
        if target != TARGET_SEGMENT and target not in self.park:
            raise LampNotAllowed("%s refused: SA%d is outside the park %s"
                                 % (describe_frame(addr, data), target, spell(self.park)))
        if addr & 1 and data in GROUP_CONFIG_OPCODES and data & GROUP_OF_OPCODE not in self.groups:
            raise LampNotAllowed("%s refused: it joins or leaves a group outside %s"
                                 % (describe_frame(addr, data), spell(self.groups)))
        return True

    def _special(self, addr, data):
        if addr in SETUP_SPECIALS:
            return
        if addr not in COMMISSIONING_SPECIALS:
            raise LampNotAllowed("special command 0x%02X refused: the virtual tier never "
                                 "sends it" % addr)
        if not self.commissioning:
            raise LampNotAllowed("commissioning frame 0x%02X refused: "
                                 "HIL_ALLOW_VIRTUAL_COMMISSIONING is off" % addr)
        if addr == INITIALISE and data != INITIALISE_UNADDRESSED:
            raise LampNotAllowed("INITIALISE 0x%02X refused: only gear without a short "
                                 "address (0x%02X) may enter initialisation"
                                 % (data, INITIALISE_UNADDRESSED))
        if addr in PARK_OPERAND_SPECIALS and data not in {(s << 1) | 1 for s in self.park}:
            raise LampNotAllowed("special command 0x%02X with operand 0x%02X refused: it "
                                 "names no park address" % (addr, data))


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


def _short_of(body):
    return body.get("short_address") if isinstance(body, dict) else None


def _apply_route(path):
    for kind, route in APPLY_ROUTES:
        match = route.fullmatch(path)
        if match:
            return kind, int(match.group(1)) if route.groups else None
    return None


def _refuse_commissioning(method, path, body):
    mode = body.get("mode") if isinstance(body, dict) else None
    if any(route.fullmatch(path) for route in COMMISSIONING_ROUTES) or (
            DISCOVERY_ROUTE.fullmatch(path) and mode not in SCAN_MODES):
        raise LampNotAllowed("%s %s %r refused: %s"
                             % (method.upper(), path, body, COMMISSIONING_REFUSAL))


def _refuse_special(addr):
    if addr in SETUP_SPECIALS:
        return
    why = COMMISSIONING_REFUSAL if addr in COMMISSIONING_SPECIALS else \
        "the guard does not know what it does to the gear"
    raise LampNotAllowed("special command 0x%02X refused: %s" % (addr, why))


def diagnostic_frame(path, body):
    if not isinstance(body, dict):
        return None
    addr = body.get("wire_address")
    if path == "dali/level" and _is_byte(addr):
        return addr & ~1, body.get("level"), None
    if path == "dali/command" and _is_byte(addr) and _is_byte(body.get("command")):
        return addr, body["command"], typed_enable(body["command"])
    frame = body.get("frame")
    if path == "dali/raw" and isinstance(frame, int) and 0 <= frame < FORWARD16_LIMIT:
        return frame >> 8, frame & BYTE, UNSEEN
    return None


def typed_enable(data):
    if data < EXTENDED_FIRST:
        return None
    return DT6 if data in DT6_OPCODES else DT8


def extended_writes(data, enabled):
    if enabled is None:
        return False
    first = EXTENDED_QUERY_FIRST.get(enabled)
    return data < first if first is not None else data != QUERY_EXTENDED_VERSION


def frame_writes(addr, data, enabled):
    if wire_target(addr) is None:
        return False
    if addr & 1 == 0 or not isinstance(data, int):
        return True
    if data >= EXTENDED_FIRST:
        return extended_writes(data, enabled)
    return data < FIRST_QUERY_OPCODE


def frame_visible(addr, data, enabled=UNSEEN):
    if wire_target(addr) is None:
        return False
    if addr & 1 == 0:
        return True
    if data >= EXTENDED_FIRST:
        return extended_visible(data, enabled)
    return data in VISIBLE_OPCODES


def extended_visible(data, enabled):
    if enabled is None:
        return False
    if enabled in VISIBLE_EXTENDED:
        return data in VISIBLE_EXTENDED[enabled]
    return extended_writes(data, enabled)


def enables(addr, data):
    return data if addr == ENABLE_DEVICE_TYPE else None


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
    runs = []
    for short in sorted(set(shorts)):
        if runs and short == runs[-1][1] + 1:
            runs[-1][1] = short
        else:
            runs.append([short, short])
    return ",".join(str(a) if a == b else "%d-%d" % (a, b) for a, b in runs) or "(none)"


def named(shorts):
    return ", ".join("SA%d" % s for s in sorted(shorts))


class LampGuard:
    def __init__(self, allowed, read_only=False, segment=None, binding=None, pending=None,
                 restart_rules=None):
        self.allowed = frozenset(allowed)
        self.read_only = bool(read_only)
        self._segment = segment
        self._binding = binding
        self._pending = pending
        self._restart_rules = restart_rules
        self.fence = None
        self._enabled = None

    @classmethod
    def for_config(cls, cfg, segment=None, binding=None, pending=None, restart_rules=None):
        return cls(cfg.lamp_short_set(), cfg.lamps_read_only, segment, binding, pending,
                   restart_rules)

    def check_request(self, method, path, body=None):
        if method.upper() in READ_METHODS:
            return []
        path = path.lstrip("/").split("?", 1)[0]
        keys = request_keys(method, path, body)
        if self.fence is not None and self.fence.check_request(method.upper(), path, body):
            return keys
        if restarts(path, body):
            self.check_restart("%s %s" % (method.upper(), path))
        _refuse_commissioning(method, path, body)
        apply = _apply_route(path)
        if apply is not None:
            return keys + self._check_apply(method, *apply)
        if path.startswith(DIAGNOSTIC_PREFIX):
            frame = diagnostic_frame(path, body)
            if frame is None:
                raise LampNotAllowed(
                    "POST %s %r refused: the lamp guard cannot tell which gear it "
                    "reaches" % (path, body))
            return self.check_frame(*frame)
        for pattern, kind, label, visible in RESOURCES:
            match = pattern.fullmatch(path)
            if match:
                key = match.group(1) if pattern.groups else None
                return keys + self._check_resource(
                    kind, key, body, label, visible(body) if callable(visible) else visible)
        return keys

    def check_restart(self, what):
        if self._restart_rules is None:
            raise LampNotAllowed("%s refused: no controller lists the owner's rules a restart "
                                 "fires" % what)
        fired = self._restart_rules()
        if fired:
            raise LampNotAllowed("%s refused: the owner's rule(s) %s fire when a controller "
                                 "starts or becomes active" % (what, ", ".join(fired)))

    def _check_resource(self, kind, key, body, label, visible):
        what = label % (key if key is not None else _short_of(body))
        if visible and self.read_only:
            raise LampNotAllowed("%s refused: HIL_LAMPS_READ_ONLY=1 forbids every visible "
                                 "action" % what)
        target = self._resource_target(kind, key, body, what)
        if target is None:
            return []
        self.check_target(target, visible, what)
        return [("shown/%d" % target, EVERY)] if kind == LAMP else []

    def _check_apply(self, method, kind, scene):
        what = "%s %s apply" % (method.upper(), kind if scene is None else "scene %d" % scene)
        if self.read_only:
            raise LampNotAllowed("%s refused: HIL_LAMPS_READ_ONLY=1 refuses an apply, which "
                                 "writes the gear of every row it changes" % what)
        if self._pending is None or self._binding is None:
            raise LampNotAllowed("%s refused: no controller lists the rows it writes" % what)
        family = "group_matrix" if scene is None else "scene_matrix/%d" % scene
        table = frozenset({"groups" if scene is None else "scenes"})
        keys = []
        for lamp in sorted(self._pending(kind, scene)):
            short = self._binding(lamp)
            if short is None:
                raise LampNotAllowed("%s refused: it writes the row of VL%d, which is bound to "
                                     "no lamp" % (what, lamp))
            self.check_target(short, False, "the %s's row of VL%d" % (what, lamp))
            keys += [("%s/%d" % (family, lamp), frozenset({"applied"})),
                     ("gear/%d" % short, table)]
        return keys

    def _resource_target(self, kind, key, body, what):
        if kind == SHORT:
            return int(key)
        if kind == SEGMENT:
            return TARGET_SEGMENT
        if kind == LAMP:
            if self._binding is None:
                raise LampNotAllowed("%s refused: no controller resolves the lamp's binding"
                                     % what)
            return self._binding(int(key))
        short = _short_of(body)
        if not _is_byte(short):
            raise LampNotAllowed("%s refused: the body names no short address, so the guard "
                                 "cannot tell which gear it reaches" % what)
        return short

    def check_frame(self, addr, data, enabled=UNSEEN):
        if enabled == UNSEEN and self._enabled is not None:
            enabled = self._enabled
        keys = self._judge_frame(addr, data, enabled)
        self._enabled = enables(addr, data)
        return keys

    def _judge_frame(self, addr, data, enabled):
        target = wire_target(addr)
        writes = target is not None and frame_writes(addr, data, enabled)
        visible = writes and frame_visible(addr, data, enabled)
        keys = frame_keys(ALL if target == TARGET_SEGMENT else target, visible) if writes else []
        if self.fence is not None and self.fence.check_frame(addr, data, enabled):
            return keys
        if SPECIAL_FIRST <= addr < BROADCAST_FIRST:
            _refuse_special(addr)
            return []
        if writes:
            self.check_target(target, visible, describe_frame(addr, data))
        return keys

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
