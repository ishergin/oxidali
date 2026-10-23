#!/usr/bin/env python3

import re
import sys
from pathlib import Path

from verify_ts_dto_mirror import load_rust, wire_fields

ROOT = Path(__file__).resolve().parent.parent

API = ROOT / "crates/dali2rust-api/src/http"
DIAGNOSTICS_DTO = API / "diagnostics_state.rs"
STATS_DTO = API / "stats_state.rs"
REDUNDANCY_DTO = API / "redundancy_state.rs"
BRIDGE = ROOT / "crates/dali2rust-adapters/src/runtime/http_bridges.rs"
FAULT_KEYS_TSX = ROOT / "web/app/src/screens/diagnostics-view.ts"
DESIGN_DOC = ROOT / "documentation/product-design/rest-api/resources/diagnostics.md"
RESOURCE_DOCS = (
    ("RedundancyStateDto", ROOT / "documentation/product-design/rest-api/resources/redundancy.md"),
    (
        "RedundancyArbitrationDto",
        ROOT / "documentation/product-design/runtime-modules/redundancy/README.md",
    ),
)
INTERNAL = ROOT / "scripts/counter_surface_internal.txt"
INTERNAL_BUDGET = ROOT / "scripts/counter_surface_internal_budget.txt"
DEFAULT_INTERNAL_BUDGET = 0
CONSUMERS = (ROOT / "tools/hil", ROOT / "tests/dali2rust-bdd/src")

DTO_FILES = (DIAGNOSTICS_DTO, STATS_DTO, REDUNDANCY_DTO)

IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]*$")

RAW_GET = r'_get\([^)]*"/api/v1/(?:diagnostics|stats|redundancy)"\)'
FETCH_CALL = re.compile(
    r"api\.diagnostics\(\)|api\.stats\(\)|diagnostics_snapshot\(|\bdiagnostics\s*[\[.]"
    r"|\.redundancy\.get\(\)|" + RAW_GET
)
FETCH = re.compile(
    r"(\w+)\s*=\s*[^=]*(?:api\.diagnostics\(\)|api\.stats\(\)|diagnostics_snapshot\("
    r"|\.redundancy\.get\(\)|" + RAW_GET + r")"
)
DEFINITION = re.compile(r"^\s*(?:def |async def |fn |async fn |pub fn |#\[)")


def dto_fields():
    structs = load_rust(ROOT, [])
    homes = {path.relative_to(ROOT).as_posix() for path in DTO_FILES}
    out = {}
    for name, struct in structs.items():
        fields = wire_fields(structs, name) if struct.loc.rsplit(":", 1)[0] in homes else None
        if fields is not None:
            out[name] = list(fields)
    return out


def rust_structs(path):
    source = path.read_text()
    out = {}
    for match in re.finditer(r"pub struct (\w+)(?:<[^>]*>)?\s*\{(.*?)\n\}", source, re.S):
        fields = re.findall(r"\n\s*pub (\w+)\s*:\s*([^,\n]+),", match.group(2))
        out[match.group(1)] = [(name, kind.strip()) for name, kind in fields]
    return out


def counter_structs():
    out = {}
    for path in sorted(ROOT.glob("crates/*/src/**/*.rs")):
        for name, fields in rust_structs(path).items():
            if not name.endswith("Counters"):
                continue
            atomic = [f for f, kind in fields if "Atomic" in kind and "&" not in kind]
            if atomic:
                out[name] = (path.relative_to(ROOT), atomic)
    return out


def read_internal():
    out = {}
    if not INTERNAL.exists():
        return out
    for line in INTERNAL.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        entry, _, reason = line.partition(" ")
        if "::" not in entry or not reason.strip():
            sys.exit("counter_surface_internal.txt: bad line %r (want `Struct::field reason`)" % line)
        struct, field = entry.split("::", 1)
        out.setdefault(struct, {})[field] = reason.strip()
    return out


def check_internal_budget(internal):
    entries = sum(len(fields) for fields in internal.values())
    try:
        budget = int(INTERNAL_BUDGET.read_text().split()[0])
    except (OSError, IndexError, ValueError):
        budget = DEFAULT_INTERNAL_BUDGET
    if entries > budget:
        return entries, [
            "counter_surface_internal.txt holds %d entries against a budget of %d "
            "(scripts/counter_surface_internal_budget.txt). An entry here is a "
            "counter nobody reads; wire it into the diagnostics/stats bridge "
            "instead of raising the number." % (entries, budget)
        ]
    if entries < budget:
        print(
            "counter surface: %d internal entries against a budget of %d — lower "
            "scripts/counter_surface_internal_budget.txt to %d to lock it in."
            % (entries, budget, entries)
        )
    return entries, []


def macro_mapped_fields(bridge_source):
    out = {}
    block = re.search(r"declare_counter_mapping! \{\n(.*)\n\}\n", bridge_source, re.S)
    if not block:
        return out
    pattern = r"\n    \w+\(\w+: ([\w:]+)\) -> \w+Dto \{\n(.*?)\n    \}"
    for src, body in re.findall(pattern, "\n" + block.group(1), re.S):
        fields = out.setdefault(src.rsplit("::", 1)[-1], set())
        for entry in body.split(","):
            entry = entry.strip()
            if entry and IDENTIFIER.match(entry.split("=")[0].strip()):
                fields.add(entry.split("=")[0].strip())
    return out


def check_counters_are_read(bridge_source, internal):
    problems = []
    mapped = macro_mapped_fields(bridge_source)
    for name, (path, fields) in sorted(counter_structs().items()):
        allowed = internal.get(name, {})
        if "*" in allowed:
            continue
        for field in fields:
            if field in mapped.get(name, ()):
                continue
            if re.search(r"\.%s\b" % re.escape(field), bridge_source):
                continue
            if field in allowed:
                continue
            problems.append(
                "%s::%s (%s) is incremented and read by no read surface — wire it "
                "into the diagnostics/stats bridge, or list it in "
                "scripts/counter_surface_internal.txt with a reason" % (name, field, path)
            )
    return problems


def payload_names(rust_types):
    names = set()
    for name, fields in rust_types.items():
        if name.endswith("Dto"):
            names.update(fields)
    return names


def consumer_keys(path):
    keys = set()
    tracked = set()
    for line in path.read_text().splitlines():
        if DEFINITION.match(line):
            tracked = set()
        match = FETCH.search(line)
        if match:
            tracked.add(match.group(1))
        touches = FETCH_CALL.search(line) is not None
        touches = touches or any(re.search(r"\b%s\s*[\[\.]" % var, line) for var in tracked)
        if not touches:
            continue
        for literal in re.findall(r'\["([^"]+)"\]|\.get\("([^"]+)"', line):
            name = literal[0] or literal[1]
            if IDENTIFIER.match(name):
                keys.add((name, line.strip()))
    return keys


def check_consumers(names):
    problems = []
    for root in CONSUMERS:
        for path in sorted(list(root.rglob("*.py")) + list(root.rglob("*.rs"))):
            if "__pycache__" in path.parts:
                continue
            for name, line in sorted(consumer_keys(path)):
                if name in names:
                    continue
                problems.append(
                    "%s: reads %r from the counter payload, which has no such key\n"
                    "      %s" % (path.relative_to(ROOT), name, line)
                )
    return problems


def check_bdd_block_list(rust_types):
    path = ROOT / "tests/dali2rust-bdd/src/steps/diagnostic_steps.rs"
    source = path.read_text()
    match = re.search(r"for block in \[(.*?)\] \{", source, re.S)
    if not match:
        return ["%s: the DIAG-030 block list was not found" % path.name]
    listed = set(re.findall(r'"([^"]+)"', match.group(1)))
    expected = set(rust_types.get("DiagnosticsDto", []))
    problems = [
        "DIAG-030 does not assert the %r block, and its own name is "
        "\"every counter block\"" % block
        for block in sorted(expected - listed)
    ]
    problems += [
        "DIAG-030 asserts a %r block the payload does not have" % block
        for block in sorted(listed - expected)
    ]
    return problems


def design_doc_blocks():
    source = DESIGN_DOC.read_text()
    head = source.find("Блоки:")
    if head < 0:
        return None
    blocks = []
    for line in source[head:].splitlines():
        if not line.startswith("|"):
            if blocks:
                break
            continue
        first = line.split("|")[1].strip()
        if first.startswith("`") and first.endswith("`") and IDENTIFIER.match(first.strip("`")):
            blocks.append(first.strip("`"))
    return blocks


def check_design_doc(rust_types):
    listed = design_doc_blocks()
    rel = DESIGN_DOC.relative_to(ROOT)
    if listed is None:
        return ["%s: the block table was not found (no `Блоки:` line)" % rel]
    expected = rust_types.get("DiagnosticsDto", [])
    problems = [
        "%s: DiagnosticsDto has a %r block the design table does not name" % (rel, block)
        for block in expected
        if block not in listed
    ]
    problems += [
        "%s: the design table names a %r block DiagnosticsDto does not have" % (rel, block)
        for block in listed
        if block not in expected
    ]
    return problems


def check_resource_docs(rust_types):
    problems = []
    for struct, doc in RESOURCE_DOCS:
        text = doc.read_text()
        if not rust_types.get(struct):
            problems.append("%s: no such DTO to check against %s" % (struct, doc.relative_to(ROOT)))
            continue
        for field in rust_types[struct]:
            if "`%s`" % field not in text:
                problems.append(
                    "%s: %s has a %r field the document does not name"
                    % (doc.relative_to(ROOT), struct, field)
                )
    return problems


def check_fault_keys(names):
    source = FAULT_KEYS_TSX.read_text()
    match = re.search(r"const FAULT_KEYS = new Set\(\[(.*?)\]\)", source, re.S)
    if not match:
        return ["%s: FAULT_KEYS not found" % FAULT_KEYS_TSX.relative_to(ROOT)]
    body = re.sub(r"//[^\n]*", "", match.group(1))
    return [
        "diagnostics-view.ts: FAULT_KEYS lists %r, which is not a field of any counter block" % key
        for key in re.findall(r"'([^']+)'", body)
        if key not in names
    ]


def main():
    try:
        rust_types = dto_fields()
    except ValueError as error:
        sys.exit("counter surface: cannot read the DTO sources: %s" % error)
    internal = read_internal()

    entries, problems = check_internal_budget(internal)
    problems += check_counters_are_read(BRIDGE.read_text(), internal)
    names = payload_names(rust_types)
    problems += check_consumers(names)
    problems += check_fault_keys(names)
    problems += check_bdd_block_list(rust_types)
    problems += check_design_doc(rust_types)
    problems += check_resource_docs(rust_types)

    if problems:
        print("counter surface: %d problem(s)\n" % len(problems))
        for problem in problems:
            print("  - %s" % problem)
        print(
            "\nThe counter surface is one list spelled in six places and only one "
            "link of it is held by the compiler. Fix the drift; do not widen the "
            "allowlist to hide it."
        )
        return 1
    print("counter surface: OK (%d internal entries)" % entries)
    return 0


if __name__ == "__main__":
    sys.exit(main())
