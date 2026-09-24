#!/usr/bin/env python3

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

API = ROOT / "crates/dali2rust-api/src/http"
DIAGNOSTICS_DTO = API / "diagnostics_state.rs"
STATS_DTO = API / "stats_state.rs"
BRIDGE = ROOT / "crates/dali2rust-adapters/src/runtime/http_bridges.rs"
TS_TYPES = ROOT / "web/app/src/api/types.ts"
FAULT_KEYS_TSX = ROOT / "web/app/src/screens/diagnostics.tsx"
DESIGN_DOC = ROOT / "documentation/product-design/rest-api/resources/diagnostics.md"
INTERNAL = ROOT / "scripts/counter_surface_internal.txt"
INTERNAL_BUDGET = ROOT / "scripts/counter_surface_internal_budget.txt"
DEFAULT_INTERNAL_BUDGET = 0
CONSUMERS = (ROOT / "tools/hil", ROOT / "tests/dali2rust-bdd/src")

ROOTS = (("DiagnosticsDto", "Diagnostics"), ("StatsReportDto", "StatsReportPayload"))

IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]*$")

FETCH_CALL = re.compile(
    r"api\.diagnostics\(\)|api\.stats\(\)|diagnostics_snapshot\(|\bdiagnostics\s*[\[.]"
)
FETCH = re.compile(r"(\w+)\s*=\s*[^=]*(?:api\.diagnostics\(\)|api\.stats\(\)|diagnostics_snapshot\()")
DEFINITION = re.compile(r"^\s*(?:def |async def |fn |async fn |pub fn |#\[)")


def rust_structs(path):
    source = path.read_text()
    out, flattened = {}, {}
    for match in re.finditer(r"pub struct (\w+)(?:<[^>]*>)?\s*\{(.*?)\n\}", source, re.S):
        body = match.group(2)
        fields = re.findall(r"\n\s*pub (\w+)\s*:\s*([^,\n]+),", body)
        flat = set(re.findall(r"#\[serde\(flatten\)\]\s*pub (\w+)\s*:", body))
        out[match.group(1)] = [(name, kind.strip()) for name, kind in fields]
        flattened[match.group(1)] = flat
    for name, fields in out.items():
        expanded = []
        for field, kind in fields:
            if field in flattened[name] and kind in out:
                expanded += out[kind]
            else:
                expanded.append((field, kind))
        out[name] = expanded
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


def ts_types():
    source = TS_TYPES.read_text()
    out = {}
    pattern = r"export (?:type (\w+) = |interface (\w+) )\{(.*?)\n\}"
    for match in re.finditer(pattern, source, re.S):
        name = match.group(1) or match.group(2)
        body = re.sub(r"/\*.*?\*/", "", match.group(3), flags=re.S)
        body = re.sub(r"//[^\n]*", "", body) + "\n"
        out[name] = [
            (field, kind.strip())
            for field, kind in re.findall(r"\n[ \t]*(\w+)\??\s*:\s*([^\n]+)(?=\n)", body)
        ]
    return out


def rust_element(kind):
    match = re.match(r"(?:Vec|Option)<(.+)>$", kind)
    return match.group(1).strip() if match else kind


def ts_element(kind):
    kind = kind.rstrip(",").strip()
    kind = re.sub(r"\s*\|\s*null$", "", kind)
    return kind[:-2].strip() if kind.endswith("[]") else kind


def compare_block(rust, ts, rust_types, ts_defs, path, problems, seen):
    if (rust, ts) in seen:
        return
    seen.add((rust, ts))
    rust_fields = dict(rust_types.get(rust, []))
    ts_fields = dict(ts_defs.get(ts, []))
    if not rust_fields:
        return
    if not ts_fields:
        problems.append("%s: Rust DTO %s has no TypeScript mirror (%s)" % (path, rust, ts))
        return
    for field in sorted(set(rust_fields) - set(ts_fields)):
        problems.append("%s.%s: in %s, missing from the TS mirror %s" % (path, field, rust, ts))
    for field in sorted(set(ts_fields) - set(rust_fields)):
        problems.append("%s.%s: in the TS mirror %s, no such field on %s" % (path, field, ts, rust))
    for field in sorted(set(rust_fields) & set(ts_fields)):
        nested = rust_element(rust_fields[field])
        if nested in rust_types:
            compare_block(
                nested, ts_element(ts_fields[field]), rust_types, ts_defs,
                "%s.%s" % (path, field), problems, seen,
            )


def payload_names(rust_types):
    names = set()
    for name, fields in rust_types.items():
        if name.endswith("Dto"):
            names.update(field for field, _ in fields)
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
    expected = {field for field, _ in rust_types.get("DiagnosticsDto", [])}
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
    expected = [field for field, _ in rust_types.get("DiagnosticsDto", [])]
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


def check_fault_keys(names):
    source = FAULT_KEYS_TSX.read_text()
    match = re.search(r"const FAULT_KEYS = new Set\(\[(.*?)\]\)", source, re.S)
    if not match:
        return ["%s: FAULT_KEYS not found" % FAULT_KEYS_TSX.relative_to(ROOT)]
    body = re.sub(r"//[^\n]*", "", match.group(1))
    return [
        "diagnostics.tsx: FAULT_KEYS lists %r, which is not a field of any counter block" % key
        for key in re.findall(r"'([^']+)'", body)
        if key not in names
    ]


def main():
    rust_types = dict(rust_structs(DIAGNOSTICS_DTO))
    rust_types.update(rust_structs(STATS_DTO))
    ts_defs = ts_types()
    internal = read_internal()

    entries, problems = check_internal_budget(internal)
    problems += check_counters_are_read(BRIDGE.read_text(), internal)
    seen = set()
    for rust_root, ts_root in ROOTS:
        compare_block(rust_root, ts_root, rust_types, ts_defs, ts_root.lower(), problems, seen)
    names = payload_names(rust_types)
    problems += check_consumers(names)
    problems += check_fault_keys(names)
    problems += check_bdd_block_list(rust_types)
    problems += check_design_doc(rust_types)

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
