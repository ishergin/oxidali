#!/usr/bin/env python3

import argparse
import re
import sys
from collections import namedtuple
from pathlib import Path

from verify_comments import lex_c_like
from verify_counter_surface import diff_fields, rust_element, ts_element

SCRIPTS = Path(__file__).resolve().parent
RUST_TREES = ("crates/dali2rust-api/src/http", "crates/dali2rust-api/src/ws")
TS_TYPES = "web/app/src/api/types.ts"
TABLE = "scripts/ts_dto_mirror.txt"
DRIFT_BUDGET = "scripts/ts_dto_mirror_drift_budget.txt"
STRUCT_MACROS = ("attribute_section_dto",)
TS_PRIMITIVES = {"string", "number", "boolean", "bigint"}
RUST_WRAPPERS = {"Option", "Vec", "Box", "Arc", "Rc", "Cow"}
DROPPED_WORDS = ("Dto", "Body", "Json", "Response", "Payload")
MIN_SHARED_WORDS = 2
MIN_PLURAL_LENGTH = 4
KNOWN_DRIFT_KINDS = ("missing_in_ts", "extra_in_ts", "required_in_ts")
TS_QUOTES = "'\""
OPENERS = "([{<"
CLOSERS = ")]}>"

ATTR = re.compile(r"#\[((?:[^\[\]]|\[[^\[\]]*\])*)\]")
ITEM = re.compile(
    r"(?P<attrs>(?:\s*#\[(?:[^\[\]]|\[[^\[\]]*\])*\])*)\s*"
    r"(?:pub(?:\([^)]*\))?\s+)?struct\s+(?P<name>\w+)\s*(?P<generics><[^{;(]*>)?\s*(?P<open>[{(;])"
)
FIELD = re.compile(
    r"\s*(?P<attrs>(?:#\[(?:[^\[\]]|\[[^\[\]]*\])*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:r#)?(?P<name>\w+)\s*:\s*(?P<kind>.+?)\s*$",
    re.S,
)
IMPL_SERIALIZE = re.compile(r"\bimpl(?:<[^>]*>)?\s+(?:serde::)?Serialize\s+for\s+(\w+)")
MACRO_RULES = re.compile(r"\bmacro_rules!\s*(\w+)\s*\{")
MEMBER = re.compile(
    r"^\s*(?:readonly\s+)?(?P<name>\w+|'[^']+')(?P<optional>\?)?\s*:\s*(?P<kind>.+?)\s*[;,]?\s*$", re.S
)
TS_HEAD = re.compile(r"^export\s+(?P<keyword>interface|type)\s+(?P<name>\w+)", re.M)
SERDE_STRING = r'\b%s\s*=\s*"([^"]*)"'

RField = namedtuple("RField", "wire kind loc optional skip flatten")
RStruct = namedtuple("RStruct", "name loc fields opaque")
TsField = namedtuple("TsField", "kind loc optional")
TsBlock = namedtuple("TsBlock", "name loc members bases opaque synthetic")
Table = namedtuple("Table", "pairs unchecked drift lines")
Ref = namedtuple("Ref", "base args")


def line_of(text, pos):
    return text.count("\n", 0, pos) + 1


def blank_comments(source, rust):
    chars = list(source)
    for comment in lex_c_like(source, rust):
        for index in range(comment.start, comment.end):
            if chars[index] != "\n":
                chars[index] = " "
    return "".join(chars)


def matching(text, start, opener="{", closer="}"):
    depth = 0
    for index in range(start, len(text)):
        if text[index] == opener:
            depth += 1
        elif text[index] == closer:
            depth -= 1
            if depth == 0:
                return index
    raise ValueError("unbalanced %r at line %d" % (opener, line_of(text, start)))


def split_top(text, separator=",", quotes='"'):
    parts, depth, start, quote = [], 0, 0, None
    for index, char in enumerate(text):
        if quote:
            quote = None if char == quote else quote
        elif char in quotes:
            quote = char
        elif char in OPENERS:
            depth += 1
        elif char in CLOSERS and not (char == ">" and text[index - 1] in "-="):
            depth -= 1
        elif char == separator and depth == 0:
            parts.append((start, text[start:index]))
            start = index + 1
    parts.append((start, text[start:]))
    return parts


def peel(strip, kind):
    while True:
        inner = strip(kind)
        if inner == kind:
            return kind
        kind = inner


def type_ref(kind, quotes='"'):
    base, _, rest = kind.partition("<")
    if not rest.endswith(">"):
        return Ref(kind.strip().split("::")[-1], ())
    parts = [part.strip() for _, part in split_top(rest[:-1], quotes=quotes)]
    args = [part for part in parts if part and not part.startswith("'")]
    return Ref(base.strip().split("::")[-1], tuple(args))


def rust_type_ref(kind):
    kind = re.sub(r"^&\s*(?:'\w+\s+)?(?:mut\s+)?", "", kind.strip())
    array = re.match(r"^\[(.+?);\s*[^;\]]+\]$", kind) or re.match(r"^\[(.+)\]$", kind)
    if array:
        return rust_type_ref(array.group(1))
    inner = rust_element(kind)
    if inner != kind:
        return rust_type_ref(inner)
    ref = type_ref(kind)
    if ref.base in RUST_WRAPPERS and ref.args:
        return rust_type_ref(ref.args[0])
    return ref


def ts_type_ref(kind):
    kind = re.sub(r"\s*\|\s*undefined$", "", kind.strip())
    return type_ref(peel(ts_element, kind), TS_QUOTES)


def serde_items(attrs):
    items = {}
    for body in ATTR.findall(attrs):
        match = re.match(r"\s*serde\s*\((.*)\)\s*$", body, re.S)
        if not match:
            continue
        for _, part in split_top(match.group(1)):
            key = re.split(r"[=(]", part.strip(), maxsplit=1)[0].strip()
            named = re.search(SERDE_STRING % "serialize", part) or re.search(r'"([^"]*)"', part)
            items[key] = named.group(1) if named else True
    return items


def apply_case(name, style):
    words = name.split("_")
    cases = {
        "lowercase": name.lower(),
        "UPPERCASE": name.upper(),
        "PascalCase": "".join(word.capitalize() for word in words),
        "camelCase": words[0] + "".join(word.capitalize() for word in words[1:]),
        "snake_case": name,
        "SCREAMING_SNAKE_CASE": name.upper(),
        "kebab-case": name.replace("_", "-"),
        "SCREAMING-KEBAB-CASE": name.upper().replace("_", "-"),
    }
    return cases.get(style, name)


def parse_fields(text, rel, body_start, body_end, container):
    fields = []
    body = text[body_start:body_end]
    for offset, part in split_top(body):
        if not part.strip():
            continue
        match = FIELD.match(part)
        if not match:
            raise ValueError("%s:%d: a field was not understood: %r" % (
                rel, line_of(text, body_start + offset), part.strip()))
        serde = serde_items(match.group("attrs"))
        raw = match.group("name")
        wire = serde["rename"] if isinstance(serde.get("rename"), str) else (
            apply_case(raw, container["rename_all"]) if isinstance(container.get("rename_all"), str) else raw)
        loc = "%s:%d" % (rel, line_of(text, body_start + offset + match.start("name")))
        fields.append(RField(
            wire, re.sub(r"\s+", " ", match.group("kind")), loc,
            "skip_serializing_if" in serde,
            "skip" in serde or "skip_serializing" in serde, "flatten" in serde,
        ))
    return fields


def struct_from_item(text, rel, match, manual):
    attrs = match.group("attrs")
    name = match.group("name")
    derived = re.search(r"derive\s*\([^)]*\bSerialize\b", attrs)
    if not derived and name not in manual:
        return None
    loc = "%s:%d" % (rel, line_of(text, match.start("name")))
    container = serde_items(attrs)
    if name in manual:
        return RStruct(name, loc, [], "a manual Serialize impl")
    if match.group("open") != "{":
        return RStruct(name, loc, [], "a tuple or unit struct")
    opaque = next((key for key in ("transparent", "into", "remote") if key in container), None)
    end = matching(text, match.end() - 1)
    fields = parse_fields(text, rel, match.end(), end, container)
    return RStruct(name, loc, fields, "serde(%s)" % opaque if opaque else None)


def macro_structs(text, rel):
    out = []
    names = "|".join(STRUCT_MACROS)
    for call in re.finditer(r"\b(?:%s)!\(\s*(\w+)\s+from\s+\w+\s*\{(.*?)\}\s*\)" % names, text, re.S):
        body_start = call.start(2)
        fields = []
        for offset, part in split_top(call.group(2)):
            if not part.strip():
                continue
            name, _, kind = part.partition(":")
            loc = "%s:%d" % (rel, line_of(text, body_start + offset + part.index(name.strip())))
            fields.append(RField(name.strip(), kind.strip(), loc, True, False, False))
        out.append(RStruct(call.group(1), "%s:%d" % (rel, line_of(text, call.start(1))), fields, None))
    return out


def unknown_macros(text, rel):
    problems = []
    for match in MACRO_RULES.finditer(text):
        body = text[match.end() - 1:matching(text, match.end() - 1)]
        if re.search(r"\bSerialize\b", body) and match.group(1) not in STRUCT_MACROS:
            problems.append(
                "%s:%d: macro %s generates a Serialize struct and verify_ts_dto_mirror.py does not "
                "expand it, so its fields are invisible" % (rel, line_of(text, match.start()), match.group(1))
            )
    return problems


def rust_file(root, path, problems):
    rel = path.relative_to(root).as_posix()
    text = blank_comments(path.read_text(encoding="utf-8"), True)
    manual = set(IMPL_SERIALIZE.findall(text))
    problems.extend(unknown_macros(text, rel))
    found = []
    for match in ITEM.finditer(text):
        struct = struct_from_item(text, rel, match, manual)
        if struct:
            found.append(struct)
    return found + macro_structs(text, rel)


def load_rust(root, problems):
    structs = {}
    for tree in RUST_TREES:
        for path in sorted((root / tree).rglob("*.rs")):
            for struct in rust_file(root, path, problems):
                if struct.name in structs:
                    problems.append("Rust struct %s is defined at %s and at %s" % (
                        struct.name, structs[struct.name].loc, struct.loc))
                structs[struct.name] = struct
    return structs


def wire_fields(structs, name, stack=()):
    struct = structs[name]
    if struct.opaque:
        return None
    out = {}
    for field in struct.fields:
        if field.skip:
            continue
        if not field.flatten:
            out[field.wire] = field
            continue
        base = rust_type_ref(field.kind).base
        inner = wire_fields(structs, base, stack + (name,)) if base in structs and base not in stack else None
        if inner is None:
            return None
        out.update(inner)
    return out


class TsParser:
    def __init__(self, rel, text):
        self.rel = rel
        self.text = text
        self.blocks = {}

    def loc(self, pos):
        return "%s:%d" % (self.rel, line_of(self.text, pos))

    def run(self):
        heads = list(TS_HEAD.finditer(self.text))
        for index, head in enumerate(heads):
            end = heads[index + 1].start() if index + 1 < len(heads) else len(self.text)
            declare = self.interface if head.group("keyword") == "interface" else self.alias
            declare(head.group("name"), head.start("name"), head.end(), end)
        return self.blocks

    def skip_generics(self, pos):
        pos = len(self.text) - len(self.text[pos:].lstrip())
        if self.text[pos] == "<":
            return matching(self.text, pos, "<", ">") + 1
        return pos

    def interface(self, name, name_pos, pos, end):
        pos = self.skip_generics(pos)
        open_at = self.text.index("{", pos)
        heritage = re.sub(r"^\s*extends\s*", "", self.text[pos:open_at])
        bases = tuple(part.strip() for _, part in split_top(heritage, quotes=TS_QUOTES) if part.strip())
        members = self.members(name, open_at)
        self.blocks[name] = TsBlock(name, self.loc(name_pos), members, bases, None, False)

    def alias(self, name, name_pos, pos, end):
        pos = self.skip_generics(pos)
        start = self.text.index("=", pos) + 1
        expression = self.text[start:end]
        parts = split_top(expression, "&", TS_QUOTES)
        if len(split_top(expression, "|", TS_QUOTES)) > 1 and len(parts) == 1:
            self.blocks[name] = TsBlock(name, self.loc(name_pos), {}, (), "a union", False)
            return
        members, bases, opaque = {}, [], None
        for offset, part in parts:
            literal = part.strip()
            if literal.startswith("{"):
                members.update(self.members(name, start + offset + part.index("{")))
            elif re.fullmatch(r"\w+", literal):
                bases.append(literal)
            else:
                opaque = "not an object type"
        self.blocks[name] = TsBlock(name, self.loc(name_pos), members, tuple(bases), opaque, False)

    def spans(self, body):
        spans, depth, start = [], 0, 0
        for index, char in enumerate(body):
            if char in "{[(":
                depth += 1
            elif char in "}])":
                depth -= 1
            elif depth == 0 and (char == ";" or (char == "\n" and self.ends_member(body, start, index))):
                spans.append((start, body[start:index]))
                start = index + 1
        spans.append((start, body[start:]))
        return [(offset, part) for offset, part in spans if part.strip()]

    @staticmethod
    def ends_member(body, start, index):
        current = body[start:index].rstrip()
        following = body[index:].lstrip()[:1]
        return bool(current) and current[-1] not in "|&:,=" and following not in ("|", "&")

    def members(self, owner, open_at):
        close = matching(self.text, open_at)
        members = {}
        for offset, part in self.spans(self.text[open_at + 1:close]):
            match = MEMBER.match(part)
            if not match:
                continue
            name = match.group("name").strip("'")
            kind_at = open_at + 1 + offset + match.start("kind")
            kind = self.hoist(owner + "." + name, kind_at, match.group("kind"))
            members[name] = TsField(kind, self.loc(open_at + 1 + offset + match.start("name")),
                                    bool(match.group("optional")))
        return members

    def hoist(self, synthetic, kind_at, kind):
        if not kind.startswith("{"):
            return re.sub(r"\s+", " ", kind)
        close = matching(self.text, kind_at)
        members = self.members(synthetic, kind_at)
        self.blocks[synthetic] = TsBlock(synthetic, self.loc(kind_at), members, (), None, True)
        return synthetic + re.sub(r"\s+", " ", self.text[close + 1:kind_at + len(kind)].rstrip(";, \n"))


def ts_fields(blocks, name, stack=()):
    block = blocks.get(name)
    if block is None or block.opaque or name in stack:
        return None
    fields = {}
    for base in block.bases:
        inner = ts_fields(blocks, base, stack + (name,))
        if inner is None:
            return None
        fields.update(inner)
    fields.update(block.members)
    return fields


def load_ts(root):
    path = root / TS_TYPES
    return TsParser(TS_TYPES, blank_comments(path.read_text(encoding="utf-8"), False)).run()


def read_table(root):
    path = root / TABLE
    table = Table({}, {}, {}, {})
    problems = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        words = raw.split()
        if not words:
            continue
        where = "%s:%d" % (TABLE, number)
        table.lines[(words[0], words[1] if len(words) > 1 else "")] = number
        if words[0] == "unchecked" and len(words) >= 3:
            table.unchecked[words[1]] = (number, " ".join(words[2:]))
        elif words[0] == "known_drift" and len(words) >= 4 and words[2] in KNOWN_DRIFT_KINDS:
            table.drift[(words[1], words[2])] = (number, " ".join(words[3:]))
        elif len(words) == 2 and words[0] not in ("unchecked", "known_drift"):
            if words[0] in table.pairs:
                problems.append("%s: %s is mapped twice" % (where, words[0]))
            table.pairs.setdefault(words[0], words[1])
        else:
            problems.append(
                "%s: bad line %r (want `RustName TsName`, `unchecked Name reason` or "
                "`known_drift RustName.field missing_in_ts|extra_in_ts|required_in_ts reason`)" % (where, raw.strip())
            )
    return table, problems


def table_line(table, kind, name):
    return "%s:%d" % (TABLE, table.lines.get((kind, name), 0))


def validate_table(table, structs, blocks):
    problems = []
    named = {name for name, block in blocks.items() if not block.synthetic}
    for rust, ts in sorted(table.pairs.items()):
        where = table_line(table, rust, ts)
        if rust not in structs:
            problems.append("%s: dead line, no serialized Rust struct %s under %s" % (
                where, rust, " or ".join(RUST_TREES)))
        elif rust in table.unchecked:
            problems.append("%s: %s is both mapped and unchecked" % (where, rust))
        if ts not in named:
            problems.append("%s: dead line, no TypeScript type %s in %s" % (where, ts, TS_TYPES))
    for name, (number, _) in sorted(table.unchecked.items()):
        if name not in structs and name not in named:
            problems.append("%s:%d: dead line, unchecked %s names no Rust struct and no TypeScript type" % (
                TABLE, number, name))
        elif name in structs and not structs[name].opaque and counterparts(name, blocks):
            problems.append("%s:%d: %s (%s) can be compared field by field and has a TypeScript "
                            "counterpart (%s); map it instead of leaving it unchecked" % (
                                TABLE, number, name, structs[name].loc, counterparts(name, blocks)[0]))
    return problems


class Mirror:
    def __init__(self, structs, blocks, table):
        self.structs = structs
        self.blocks = blocks
        self.table = table
        self.seen = set()
        self.fields = 0
        self.problems = []
        self.mismatches = {}

    def where(self, side, name, field):
        if side == "rust":
            return wire_fields(self.structs, name)[field].loc if field else self.structs[name].loc
        return ts_fields(self.blocks, name)[field].loc if field else self.blocks[name].loc

    def walk(self, rust, ts, path):
        if (rust, ts) in self.seen or rust in self.table.unchecked:
            return
        self.seen.add((rust, ts))
        rust_fields = wire_fields(self.structs, rust)
        mirror_fields = ts_fields(self.blocks, ts)
        if rust_fields is None or mirror_fields is None:
            side, why = (rust, self.structs[rust].opaque) if rust_fields is None else (ts, self.blocks[ts].opaque)
            self.problems.append("%s: %s cannot be compared field by field (%s); list it as `unchecked %s "
                                 "reason` in %s" % (path, side, why or "a flatten it cannot follow", side, TABLE))
            return
        self.fields += len(rust_fields)
        kinds = {name: field.kind for name, field in rust_fields.items()}
        mirror_kinds = {name: field.kind for name, field in mirror_fields.items()}
        for kind, field, message in diff_fields(rust, ts, kinds, mirror_kinds, path, self.where):
            self.report(rust, kind, field, message)
        for field in sorted(set(rust_fields) & set(mirror_fields)):
            self.required(rust, ts, field, rust_fields[field], mirror_fields[field], path)
            self.nested(rust_fields[field].kind, mirror_fields[field].kind, "%s.%s" % (path, field))

    def required(self, rust, ts, field, rust_field, ts_field, path):
        if rust_field.optional and not ts_field.optional and "undefined" not in ts_field.kind:
            self.report(rust, "required_in_ts", field, (
                "%s.%s: Rust %s (%s) omits the field when empty (skip_serializing_if), but the TS mirror %s "
                "(%s) declares it required" % (path, field, rust, rust_field.loc, ts, ts_field.loc)))

    def report(self, rust, kind, field, message):
        key = ("%s.%s" % (rust, field), kind)
        if key in self.table.drift:
            self.mismatches[key] = message
        else:
            self.problems.append(message)
            self.mismatches[key] = None

    def nested(self, rust_kind, ts_kind, path):
        rust_ref = rust_type_ref(rust_kind)
        ts_ref = ts_type_ref(ts_kind)
        if rust_ref.base in self.structs:
            self.nested_pair(rust_ref.base, ts_ref.base, path)
        for rust_arg, ts_arg in zip(rust_ref.args, ts_ref.args):
            self.nested(rust_arg, ts_arg, path)

    def nested_pair(self, rust, ts, path):
        if rust in self.table.unchecked:
            return
        mapped = self.table.pairs.get(rust)
        block = self.blocks.get(ts)
        if block is None:
            if ts in TS_PRIMITIVES:
                self.problems.append("%s: Rust %s is a DTO (%s), TypeScript types it %s" % (
                    path, rust, self.structs[rust].loc, ts))
            return
        if mapped and mapped != ts:
            self.problems.append("%s: Rust %s is mapped to %s in %s, but the TypeScript field is typed %s" % (
                path, rust, mapped, TABLE, ts))
        elif not mapped and not block.synthetic:
            self.problems.append("%s: Rust %s (%s) is typed %s in TypeScript (%s); add `%s %s` to %s" % (
                path, rust, self.structs[rust].loc, ts, block.loc, rust, ts, TABLE))
        else:
            self.walk(rust, ts, path)


def words(name):
    name = name.split(".")[0]
    found = re.findall(r"[A-Z][a-z0-9]*", name)
    while len(found) > 1 and found[-1] in DROPPED_WORDS:
        found.pop()
    return [word[:-1] if word.endswith("s") and len(word) >= MIN_PLURAL_LENGTH else word for word in found]


def counterparts(rust, blocks):
    target = words(rust)
    found = []
    for name, block in sorted(blocks.items()):
        if block.synthetic:
            continue
        other = words(name)
        short, long = sorted((target, other), key=len)
        if target == other or (len(short) >= MIN_SHARED_WORDS and long[-len(short):] == short):
            found.append(name)
    return found


def flattened(structs, name):
    out = set()
    for field in structs[name].fields:
        base = rust_type_ref(field.kind).base
        if field.flatten and base in structs:
            out |= {base} | flattened(structs, base)
    return out


def covered_structs(mirror):
    covered = {rust for rust, _ in mirror.seen}
    for rust in list(covered):
        covered |= flattened(mirror.structs, rust)
    return covered


def unmapped(mirror):
    covered = covered_structs(mirror)
    problems = []
    for name, struct in sorted(mirror.structs.items()):
        if name in covered or name in mirror.table.unchecked:
            continue
        near = counterparts(name, mirror.blocks)
        if struct.opaque and name not in mirror.table.pairs:
            problems.append(
                "%s: Rust %s serialises through %s, which this gate cannot follow; add `unchecked %s reason` "
                "to %s" % (struct.loc, name, struct.opaque, name, TABLE))
        elif near:
            problems.append(
                "%s: Rust %s has a TypeScript counterpart by name (%s at %s) and no line in %s; add "
                "`%s %s`, or `unchecked %s reason`" % (
                    struct.loc, name, near[0], mirror.blocks[near[0]].loc, TABLE, name, near[0], name))
    return problems


def stale_drift(mirror):
    return [
        "%s:%d: known_drift %s %s matches no current mismatch; remove the line" % (TABLE, number, name, kind)
        for (name, kind), (number, _) in sorted(mirror.table.drift.items(), key=lambda item: item[1][0])
        if (name, kind) not in mirror.mismatches
    ]


def drift_budget(root, entries):
    try:
        budget = int((root / DRIFT_BUDGET).read_text(encoding="utf-8").split()[0])
    except (OSError, IndexError, ValueError):
        return ["%s is missing, empty or not an integer" % DRIFT_BUDGET]
    if entries > budget:
        return ["%s holds %d known_drift line(s) against a budget of %d (%s); fix the drift instead of "
                "recording it" % (TABLE, entries, budget, DRIFT_BUDGET)]
    if entries < budget:
        return ["%s is %d but %s holds %d known_drift line(s); lower the budget to lock it in" % (
            DRIFT_BUDGET, budget, TABLE, entries)]
    return []


def check(root):
    problems = []
    structs = load_rust(root, problems)
    blocks = load_ts(root)
    table, table_problems = read_table(root)
    problems += table_problems + validate_table(table, structs, blocks)
    mirror = Mirror(structs, blocks, table)
    for rust, ts in sorted(table.pairs.items()):
        if rust in structs and ts in blocks:
            mirror.walk(rust, ts, ts)
    problems += mirror.problems + unmapped(mirror) + stale_drift(mirror)
    problems += drift_budget(root, len(table.drift))
    return problems, mirror


def inventory(mirror):
    covered = covered_structs(mirror)
    mirrored = {ts for _, ts in mirror.seen}
    print("unchecked:")
    for name, (_, reason) in sorted(mirror.table.unchecked.items()):
        print("  %s: %s" % (name, reason))
    print("Rust structs outside the table with no TypeScript counterpart:")
    for name, struct in sorted(mirror.structs.items()):
        if name not in covered and name not in mirror.table.unchecked:
            print("  %s (%s)" % (name, struct.loc))
    print("TypeScript object types no Rust struct mirrors:")
    for name, block in sorted(mirror.blocks.items()):
        if name not in mirrored and name not in mirror.table.unchecked and not block.synthetic and not block.opaque:
            print("  %s (%s)" % (name, block.loc))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=SCRIPTS.parent)
    parser.add_argument("--list", action="store_true")
    arguments = parser.parse_args()
    try:
        problems, mirror = check(arguments.root.resolve())
    except (OSError, ValueError) as error:
        print("verify_ts_dto_mirror: FAIL (cannot read the sources: %s)" % error, file=sys.stderr)
        return 1
    if arguments.list:
        inventory(mirror)
    if problems:
        print("\n".join("  - %s" % problem for problem in problems), file=sys.stderr)
        print("verify_ts_dto_mirror: FAIL (%d problem(s))" % len(problems), file=sys.stderr)
        return 1
    tolerated = len(mirror.table.drift)
    note = "; %d known drift" % tolerated if tolerated else ""
    print("verify_ts_dto_mirror: OK (%d pairs, %d fields%s)" % (len(mirror.seen), mirror.fields, note))
    return 0


if __name__ == "__main__":
    sys.exit(main())
