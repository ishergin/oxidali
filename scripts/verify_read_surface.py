#!/usr/bin/env python3
import re
import sys
from pathlib import Path

ROOT = str(Path(__file__).resolve().parent.parent)


def dto_blocks(path: str, struct: str) -> list[str]:
    src = open(path, encoding="utf-8").read()
    m = re.search(rf"pub struct {struct}\s*\{{(.*?)\n\}}", src, re.S)
    if not m:
        sys.exit(f"verify_read_surface: {struct} not found in {path}")
    return re.findall(r"^\s*pub (\w+):", m.group(1), re.M)


def screen_source(path: str) -> str:
    return open(path, encoding="utf-8").read()


def allowlist() -> dict[str, str]:
    out: dict[str, str] = {}
    for line in open(f"{ROOT}/scripts/read_surface_internal.txt", encoding="utf-8"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, _, reason = line.partition(" ")
        out[name] = reason.strip()
    return out


SURFACES = [
    (
        "stats",
        "crates/dali2rust-api/src/http/stats_state.rs",
        "StatsReportDto",
        "web/app/src/screens/stats.tsx",
    ),
    (
        "diagnostics",
        "crates/dali2rust-api/src/http/diagnostics_state.rs",
        "DiagnosticsDto",
        "web/app/src/screens/diagnostics.tsx",
    ),
]

allowed = allowlist()
budget = int(open(f"{ROOT}/scripts/read_surface_internal_budget.txt", encoding="utf-8").read().strip())
fail: list[str] = []

if len(allowed) > budget:
    fail.append(
        f"the exception list grew: {len(allowed)} entries against a frozen {budget}.\n"
        "     That file only ever shrinks — render the block instead of naming it."
    )

checked = 0
known: set[str] = set()
stale: list[str] = []
for surface, dto_path, struct, screen_path in SURFACES:
    blocks = dto_blocks(f"{ROOT}/{dto_path}", struct)
    src = screen_source(f"{ROOT}/{screen_path}")
    for block in blocks:
        checked += 1
        key = f"{surface}.{block}"
        known.add(key)
        rendered = bool(re.search(rf"\bdata\.{re.escape(block)}\b", src))
        if key in allowed:
            if rendered:
                stale.append(
                    f"{key} is in the exception list and IS rendered by {screen_path}.\n"
                    "     Drop the line and lower scripts/read_surface_internal_budget.txt."
                )
            continue
        if rendered:
            continue
        fail.append(
            f"{key} is published by {struct} and reached no screen "
            f"({screen_path}).\n"
            "     Render it, or name it in scripts/read_surface_internal.txt with a reason."
        )

for key in sorted(set(allowed) - known):
    stale.append(
        f"{key} is in the exception list and is not a block of any payload here.\n"
        "     A renamed or deleted field left it behind; drop the line and lower the budget."
    )

if stale:
    fail.extend(stale)

if fail:
    print("verify_read_surface: a read payload has a block nothing shows.\n", file=sys.stderr)
    for f in fail:
        print("  " + f + "\n", file=sys.stderr)
    sys.exit(1)

print(
    f"verify_read_surface: OK ({checked} blocks across {len(SURFACES)} payloads, "
    f"{len(allowed)} documented exceptions of {budget})"
)
