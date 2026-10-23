#!/usr/bin/env python3

import argparse
import re
import sys
from collections import namedtuple
from pathlib import Path

from verify_comments import lex_c_like

SCRIPTS = Path(__file__).resolve().parent
RUST_TREES = ("crates/dali2rust-api/src/http", "crates/dali2rust-api/src/ws")
RUST_SOURCES = "crates/*/src/**/*.rs"
TS_TYPES = "web/app/src/api/types.ts"
TABLE = "scripts/ts_dto_mirror.txt"
DRIFT_BUDGET = "scripts/ts_dto_mirror_drift_budget.txt"
STRUCT_MACROS = ("attribute_section_dto",)
TYPE_PLACEHOLDER = "$ty"
TS_PRIMITIVES = {"string", "number", "boolean", "bigint"}
RUST_SCALARS = {
    **{name: "number" for name in (
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64")},
    "bool": "boolean", "String": "string", "str": "string", "char": "string",
}
RUST_WRAPPERS = {"Option", "Vec", "Box", "Arc", "Rc", "Cow", "MaybeObj", "MaybeSeq"}
RUST_CATCH_ALLS = {"BTreeMap", "HashMap", "IndexMap", "Map"}
TS_WRAPPERS = {"Partial", "Required", "Readonly", "NonNullable"}
TS_NULLABLE = {"null", "unknown", "any"}
DROPPED_WORDS = ("Dto", "Body", "Json", "Response", "Payload")
MIN_SHARED_WORDS = 2
MIN_PLURAL_LENGTH = 4
KNOWN_DRIFT_KINDS = ("missing_in_ts", "extra_in_ts", "required_in_ts", "non_nullable_in_ts")
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
MACRO_FIELD = re.compile(
    r"\$\(\s*(?P<attrs>(?:#\[(?:[^\[\]]|\[[^\[\]]*\])*\]\s*)*)pub\s+\$\w+\s*:\s*(?P<kind>[^,]+?)\s*,?\s*\)\+"
)
STRUCT_NAME = re.compile(r"\bstruct\s+(\w+)")
MEMBER = re.compile(
    r"^\s*(?:readonly\s+)?(?P<name>\w+|'[^']+')(?P<optional>\?)?\s*:\s*(?P<kind>.+?)\s*[;,]?\s*$", re.S
)
TS_HEAD = re.compile(r"^export\s+(?P<keyword>interface|type)\s+(?P<name>\w+)", re.M)
TS_RECORD = re.compile(r"Record\s*<\s*string\s*,(.*)>", re.S)
SERDE_STRING = r'\b%s\s*=\s*"([^"]*)"'

RField = namedtuple("RField", "wire kind loc optional skip flatten")
RStruct = namedtuple("RStruct", "name loc fields opaque direction generated")
TsField = namedtuple("TsField", "kind loc optional")
TsBlock = namedtuple("TsBlock", "name loc members bases opaque synthetic record defaults")
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


def rust_element(kind):
    match = re.match(r"(?:Vec|Option)<(.+)>$", kind)
    return match.group(1).strip() if match else kind


def ts_element(kind):
    kind = kind.rstrip(",").strip()
    kind = re.sub(r"\s*\|\s*null$", "", kind)
    return kind[:-2].strip() if kind.endswith("[]") else kind


def diff_fields(rust, ts, rust_fields, ts_fields, path, where=None):
    def at(side, name, field):
        return " (%s)" % where(side, name, field) if where else ""

    out = []
    for field in sorted(set(rust_fields) - set(ts_fields)):
        out.append((
            "missing_in_ts", field,
            "%s.%s: in %s%s, missing from the TS mirror %s%s"
            % (path, field, rust, at("rust", rust, field), ts, at("ts", ts, None)),
        ))
    for field in sorted(set(ts_fields) - set(rust_fields)):
        out.append((
            "extra_in_ts", field,
            "%s.%s: in the TS mirror %s%s, no such field on %s%s"
            % (path, field, ts, at("ts", ts, field), rust, at("rust", rust, None)),
        ))
    return out


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
    ref = type_ref(peel(ts_element, kind), TS_QUOTES)
    if ref.base in TS_WRAPPERS and ref.args:
        return ts_type_ref(ref.args[0])
    return ref


def ts_accepts_null(kind):
    return any(part.strip() in TS_NULLABLE for _, part in split_top(kind, "|", TS_QUOTES))


def naming_of(direction):
    return "deserialize" if direction == "de" else "serialize"


def serde_value(part, naming):
    if "(" in part:
        named = re.search(SERDE_STRING % naming, part)
    else:
        named = re.search(r'=\s*"([^"]*)"', part)
    return named.group(1) if named else True


def serde_items(attrs, naming="serialize"):
    items = {}
    for body in ATTR.findall(attrs):
        match = re.match(r"\s*serde\s*\((.*)\)\s*$", body, re.S)
        if not match:
            continue
        for _, part in split_top(match.group(1)):
            key = re.split(r"[=(]", part.strip(), maxsplit=1)[0].strip()
            items[key] = serde_value(part, naming)
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


def wire_name(raw, serde, container):
    if isinstance(serde.get("rename"), str):
        return serde["rename"]
    style = container.get("rename_all")
    return apply_case(raw, style) if isinstance(style, str) else raw


def parse_fields(text, rel, body_start, body_end, container, direction):
    fields = []
    skips = ("skip", "skip_deserializing") if direction == "de" else ("skip", "skip_serializing")
    for offset, part in split_top(text[body_start:body_end]):
        if not part.strip():
            continue
        match = FIELD.match(part)
        if not match:
            raise ValueError("%s:%d: a field was not understood: %r" % (
                rel, line_of(text, body_start + offset), part.strip()))
        serde = serde_items(match.group("attrs"), naming_of(direction))
        absent = "default" if direction == "de" else "skip_serializing_if"
        loc = "%s:%d" % (rel, line_of(text, body_start + offset + match.start("name")))
        fields.append(RField(
            wire_name(match.group("name"), serde, container),
            re.sub(r"\s+", " ", match.group("kind")), loc,
            absent in serde or (direction == "de" and "default" in container),
            any(key in serde for key in skips), "flatten" in serde,
        ))
    return fields


def derive_direction(attrs, name, manual):
    derives = " ".join(re.findall(r"derive\s*\(([^)]*)\)", attrs))
    serialized = name in manual or re.search(r"\bSerialize\b", derives) is not None
    deserialized = re.search(r"\bDeserialize\b", derives) is not None
    if serialized and deserialized:
        return "both"
    return "ser" if serialized else "de" if deserialized else None


def struct_from_item(text, rel, match, manual, names):
    attrs = match.group("attrs")
    name = match.group("name")
    direction = derive_direction(attrs, name, manual)
    if direction is None or (names is not None and name not in names):
        return None
    loc = "%s:%d" % (rel, line_of(text, match.start("name")))
    container = serde_items(attrs, naming_of(direction))
    if name in manual:
        return RStruct(name, loc, [], "a manual Serialize impl", direction, False)
    if match.group("open") != "{":
        return RStruct(name, loc, [], "a tuple or unit struct", direction, False)
    opaque = next((key for key in ("transparent", "into", "remote") if key in container), None)
    end = matching(text, match.end() - 1)
    fields = parse_fields(text, rel, match.end(), end, container, direction)
    return RStruct(name, loc, fields, "serde(%s)" % opaque if opaque else None, direction, False)


def macro_templates(paths, problems):
    templates = {}
    for path in paths:
        text = blank_comments(path.read_text(encoding="utf-8"), True)
        for match in MACRO_RULES.finditer(text):
            if match.group(1) in STRUCT_MACROS:
                field = MACRO_FIELD.search(text, match.end(), matching(text, match.end() - 1))
                if field:
                    optional = "skip_serializing_if" in serde_items(field.group("attrs"))
                    templates[match.group(1)] = (field.group("kind"), optional)
    for name in STRUCT_MACROS:
        if name not in templates:
            problems.append("macro %s has no `pub $field: <type>` template under %s that verify_ts_dto_mirror.py "
                            "can read" % (name, " or ".join(RUST_TREES)))
    return templates


def macro_structs(text, rel, templates, names):
    out = []
    for name, (template, optional) in templates.items():
        for call in re.finditer(r"\b%s!\(\s*(\w+)\s+from\s+\w+\s*\{(.*?)\}\s*\)" % name, text, re.S):
            if names is not None and call.group(1) not in names:
                continue
            fields = []
            for offset, part in split_top(call.group(2)):
                if not part.strip():
                    continue
                field, _, kind = part.partition(":")
                loc = "%s:%d" % (rel, line_of(text, call.start(2) + offset + part.index(field.strip())))
                fields.append(RField(
                    field.strip(), template.replace(TYPE_PLACEHOLDER, kind.strip()), loc, optional, False, False))
            loc = "%s:%d" % (rel, line_of(text, call.start(1)))
            out.append(RStruct(call.group(1), loc, fields, None, "ser", True))
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


def rust_file(root, path, problems, names=None, templates=None, strict=False):
    rel = path.relative_to(root).as_posix()
    text = blank_comments(path.read_text(encoding="utf-8"), True)
    manual = set(IMPL_SERIALIZE.findall(text))
    if strict:
        problems.extend(unknown_macros(text, rel))
    found = []
    for match in ITEM.finditer(text):
        struct = struct_from_item(text, rel, match, manual, names)
        if struct:
            found.append(struct)
    return found + macro_structs(text, rel, templates or {}, names)


def add_struct(structs, struct, problems):
    if struct.name in structs:
        problems.append("Rust struct %s is defined at %s and at %s" % (
            struct.name, structs[struct.name].loc, struct.loc))
    structs[struct.name] = struct


def struct_index(root, taken):
    index = {}
    for path in sorted(root.glob(RUST_SOURCES)):
        if path in taken:
            continue
        for name in set(STRUCT_NAME.findall(path.read_text(encoding="utf-8"))):
            index.setdefault(name, []).append(path)
    return index


def referenced_names(structs):
    return {word for struct in structs for field in struct.fields for word in re.findall(r"\b[A-Z]\w*", field.kind)}


def load_named(root, structs, wanted, taken, templates, problems):
    index = struct_index(root, taken)
    tried = set(structs)
    pending = (wanted & set(index)) - tried
    while pending:
        tried |= pending
        by_file = {}
        for name in sorted(pending):
            for path in index[name]:
                by_file.setdefault(path, set()).add(name)
        loaded = []
        for path, names in sorted(by_file.items()):
            loaded += rust_file(root, path, problems, names, templates)
        for struct in loaded:
            add_struct(structs, struct, problems)
        pending = (referenced_names(loaded) & set(index)) - tried


def load_rust(root, problems, wanted=()):
    structs = {}
    paths = [path for tree in RUST_TREES for path in sorted((root / tree).rglob("*.rs"))]
    templates = macro_templates(paths, problems)
    for path in paths:
        for struct in rust_file(root, path, problems, None, templates, True):
            add_struct(structs, struct, problems)
    load_named(root, structs, set(wanted), set(paths), templates, problems)
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
        if struct.direction == "de" and base in RUST_CATCH_ALLS:
            continue
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

    def generics(self, pos):
        pos = len(self.text) - len(self.text[pos:].lstrip())
        if self.text[pos] != "<":
            return pos, ()
        close = matching(self.text, pos, "<", ">")
        params = split_top(self.text[pos + 1:close], quotes=TS_QUOTES)
        return close + 1, tuple(part.partition("=")[2].strip() or None for _, part in params)

    def interface(self, name, name_pos, pos, end):
        pos, defaults = self.generics(pos)
        open_at = self.text.index("{", pos)
        heritage = re.sub(r"^\s*extends\s*", "", self.text[pos:open_at])
        bases = tuple(part.strip() for _, part in split_top(heritage, quotes=TS_QUOTES) if part.strip())
        members = self.members(name, open_at)
        self.blocks[name] = TsBlock(name, self.loc(name_pos), members, bases, None, False, None, defaults)

    def alias(self, name, name_pos, pos, end):
        pos, defaults = self.generics(pos)
        start = self.text.index("=", pos) + 1
        expression = self.text[start:end]
        record = TS_RECORD.fullmatch(expression.strip(" \n;"))
        if record:
            self.blocks[name] = TsBlock(name, self.loc(name_pos), {}, (), None, False, record.group(1).strip(), defaults)
            return
        parts = split_top(expression, "&", TS_QUOTES)
        if len(split_top(expression, "|", TS_QUOTES)) > 1 and len(parts) == 1:
            self.blocks[name] = TsBlock(name, self.loc(name_pos), {}, (), "a union", False, None, defaults)
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
        self.blocks[name] = TsBlock(name, self.loc(name_pos), members, tuple(bases), opaque, False, None, defaults)

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
        self.blocks[synthetic] = TsBlock(synthetic, self.loc(kind_at), members, (), None, True, None, ())
        return synthetic + re.sub(r"\s+", " ", self.text[close + 1:kind_at + len(kind)].rstrip(";, \n"))


def ts_fields(blocks, name, stack=()):
    block = blocks.get(name)
    if block is None or block.opaque or block.record or name in stack:
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
    order = []
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
            order.append((number, words[0]))
        else:
            problems.append(
                "%s: bad line %r (want `RustName TsName`, `RustScalar string|number|boolean`, `unchecked Name "
                "reason` or `known_drift RustName.field %s reason`)" % (
                    where, raw.strip(), "|".join(KNOWN_DRIFT_KINDS))
            )
    problems += [
        "%s:%d: %s is out of alphabetical order (it follows %s); keep the pairs sorted" % (TABLE, after[0], after[1], before[1])
        for before, after in zip(order, order[1:]) if after[1] < before[1]
    ]
    return table, problems


def table_line(table, kind, name):
    return "%s:%d" % (TABLE, table.lines.get((kind, name), 0))


def validate_table(table, structs, blocks):
    problems = []
    named = {name for name, block in blocks.items() if not block.synthetic}
    for rust, ts in sorted(table.pairs.items()):
        where = table_line(table, rust, ts)
        if ts in TS_PRIMITIVES:
            if rust in structs:
                problems.append("%s: %s is a serialized struct (%s), it cannot be mapped to the scalar %s" % (
                    where, rust, structs[rust].loc, ts))
            continue
        if rust not in structs:
            problems.append("%s: dead line, no serialized Rust struct %s under %s or in the structs they reach" % (
                where, rust, " or ".join(RUST_TREES)))
        elif rust in table.unchecked:
            problems.append("%s: %s is both mapped and unchecked" % (where, rust))
        if ts not in named:
            problems.append("%s: dead line, no TypeScript type %s in %s" % (where, ts, TS_TYPES))
    for name, (number, _) in sorted(table.unchecked.items()):
        if name not in structs and name not in named:
            problems.append("%s:%d: dead line, unchecked %s names no Rust struct and no TypeScript type" % (
                TABLE, number, name))
        elif name in structs and not structs[name].opaque and structs[name].direction != "de" \
                and counterparts(name, blocks):
            problems.append("%s:%d: %s (%s) can be compared field by field and has a TypeScript "
                            "counterpart (%s); map it instead of leaving it unchecked" % (
                                TABLE, number, name, structs[name].loc, counterparts(name, blocks)[0]))
    return problems


class Mirror:
    def __init__(self, structs, blocks, table):
        self.structs = structs
        self.blocks = blocks
        self.table = table
        self.scalars = {**RUST_SCALARS, **{rust: ts for rust, ts in table.pairs.items() if ts in TS_PRIMITIVES}}
        self.used_scalars = set()
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
        if rust_fields is not None and self.blocks[ts].record:
            self.walk_record(rust_fields, self.blocks[ts].record, path)
            return
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
            self.nullable(rust, ts, field, rust_fields[field], mirror_fields[field], path)
            self.nested(rust_fields[field].kind, mirror_fields[field].kind, "%s.%s" % (path, field))

    def walk_record(self, rust_fields, value, path):
        self.fields += len(rust_fields)
        for name, field in sorted(rust_fields.items()):
            self.nested(field.kind, value, "%s.%s" % (path, name))

    def required(self, rust, ts, field, rust_field, ts_field, path):
        if self.structs[rust].direction == "de":
            return
        if rust_field.optional and not ts_field.optional and "undefined" not in ts_field.kind:
            self.report(rust, "required_in_ts", field, (
                "%s.%s: Rust %s (%s) omits the field when empty (skip_serializing_if), but the TS mirror %s "
                "(%s) declares it required" % (path, field, rust, rust_field.loc, ts, ts_field.loc)))

    def nullable(self, rust, ts, field, rust_field, ts_field, path):
        if self.structs[rust].direction == "de" or rust_field.optional or type_ref(rust_field.kind).base != "Option":
            return
        if not ts_field.optional and not ts_accepts_null(ts_field.kind):
            self.report(rust, "non_nullable_in_ts", field, (
                "%s.%s: Rust %s (%s) serialises the field as null when it is empty (an Option without "
                "skip_serializing_if), but the TS mirror %s (%s) declares it neither optional nor `| null`" % (
                    path, field, rust, rust_field.loc, ts, ts_field.loc)))

    def report(self, rust, kind, field, message):
        key = ("%s.%s" % (rust, field), kind)
        if key in self.table.drift:
            self.mismatches[key] = message
        else:
            self.problems.append(message)
            self.mismatches[key] = None

    def arguments(self, rust_ref, ts_ref, path):
        block = self.blocks.get(ts_ref.base)
        if block is None or rust_ref.base not in self.structs:
            return zip(rust_ref.args, ts_ref.args)
        supplied = list(ts_ref.args)
        supplied += [default for default in block.defaults[len(supplied):] if default]
        if len(rust_ref.args) != len(supplied):
            self.problems.append("%s: Rust %s (%s) takes %d type argument(s), the TypeScript %s (%s) is given %d" % (
                path, rust_ref.base, self.structs[rust_ref.base].loc, len(rust_ref.args), ts_ref.base,
                block.loc, len(supplied)))
        return zip(rust_ref.args, supplied)

    def nested(self, rust_kind, ts_kind, path, argument=False):
        rust_ref = rust_type_ref(rust_kind)
        ts_ref = ts_type_ref(ts_kind)
        if rust_ref.base in self.structs:
            self.nested_pair(rust_ref.base, ts_ref.base, path)
        elif rust_ref.base in self.scalars:
            self.scalar_pair(rust_ref.base, ts_ref.base, path)
        for rust_arg, ts_arg in self.arguments(rust_ref, ts_ref, path):
            self.nested(rust_arg, ts_arg, path, True)

    def scalar_pair(self, rust, ts, path):
        expected = self.scalars[rust]
        self.used_scalars.add(rust)
        block = self.blocks.get(ts)
        if ts in TS_PRIMITIVES and ts != expected:
            self.problems.append("%s: Rust %s serialises as a %s, TypeScript types it %s" % (
                path, rust, expected, ts))
        elif block is not None and not block.opaque and not block.record:
            self.problems.append("%s: Rust %s serialises as a %s, TypeScript types it %s (%s)" % (
                path, rust, expected, ts, block.loc))

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
        shares = len(short) >= MIN_SHARED_WORDS and short in (long[:len(short)], long[-len(short):])
        if target == other or shares:
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


def unresolved_generated(mirror):
    unresolved = {}
    for name, struct in sorted(mirror.structs.items()):
        for field in struct.fields if struct.generated else ():
            for argument in rust_type_ref(field.kind).args:
                base = rust_type_ref(argument).base
                if base not in mirror.structs and base not in mirror.scalars:
                    unresolved.setdefault(base, "%s.%s (%s)" % (name, field.wire, field.loc))
    return [
        "%s is generated into %s with a type that is neither a serialized struct nor a scalar; add `%s "
        "string|number|boolean` or `%s <TsName>` to %s" % (base, first, base, base, TABLE)
        for base, first in sorted(unresolved.items())
    ]


def stale_scalars(mirror):
    return [
        "%s: dead line, the scalar %s is never the type of a field or type argument the mirror reaches" % (
            table_line(mirror.table, rust, ts), rust)
        for rust, ts in sorted(mirror.table.pairs.items())
        if ts in TS_PRIMITIVES and rust not in mirror.used_scalars
    ]


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
    table, problems = read_table(root)
    structs = load_rust(root, problems, table.pairs)
    blocks = load_ts(root)
    problems += validate_table(table, structs, blocks)
    mirror = Mirror(structs, blocks, table)
    for rust, ts in sorted(table.pairs.items()):
        if rust in structs and ts in blocks:
            mirror.walk(rust, ts, ts)
    problems += mirror.problems + unmapped(mirror) + unresolved_generated(mirror)
    problems += stale_scalars(mirror) + stale_drift(mirror)
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
        if name not in mirrored and name not in mirror.table.unchecked and not block.synthetic \
                and not block.opaque and not block.record:
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
