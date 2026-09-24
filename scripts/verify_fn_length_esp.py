#!/usr/bin/env python3

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUDGET_FILE = ROOT / "scripts" / "fn_length_esp_budget.txt"

THRESHOLD = 40

ESPIDF_CFG = 'target_os = "espidf"'
CONTENT_SCAN_GLOB = "crates/*/src/**/*.rs"

SCAN_GLOBS = (
    "crates/*/src/**/esp_idf.rs",
    "crates/*/src/**/esp_ws.rs",
    "crates/dali2rust-firmware/src/**/*.rs",
    "crates/dali2rust-bsp/src/**/*.rs",
    "crates/dali2rust-p4-bringup/src/**/*.rs",
)

FN_RE = re.compile(
    r"""^\s*
    (?:pub(?:\([^)]*\))?\s+)?
    (?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?
    (?:extern\s+"[^"]*"\s+)?
    fn\s+(?P<name>\w+)
    """,
    re.VERBOSE,
)


def scan_file(path: Path) -> list[tuple[str, int, int]]:
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    findings: list[tuple[str, int, int]] = []
    i = 0
    while i < len(lines):
        match = FN_RE.match(lines[i])
        if not match:
            i += 1
            continue

        depth = 0
        started = False
        body_lines = 0
        in_block_comment = False
        j = i
        while j < len(lines):
            line = lines[j]
            for ch in line:
                if ch == "{":
                    depth += 1
                    started = True
                elif ch == "}":
                    depth -= 1

            if started and j > i:
                stripped = line.strip()
                if in_block_comment:
                    if "*/" in stripped:
                        in_block_comment = False
                elif stripped.startswith("/*"):
                    if "*/" not in stripped:
                        in_block_comment = True
                elif stripped and not stripped.startswith("//"):
                    body_lines += 1

            if started and depth == 0:
                break
            j += 1

        if body_lines > THRESHOLD:
            findings.append((match.group("name"), i + 1, body_lines))
        i = j + 1
    return findings


def scanned_files() -> list[Path]:
    seen: set[Path] = set()
    for pattern in SCAN_GLOBS:
        for path in ROOT.glob(pattern):
            if "target" not in path.parts:
                seen.add(path)
    for path in ROOT.glob(CONTENT_SCAN_GLOB):
        if path in seen or "target" in path.parts:
            continue
        if ESPIDF_CFG in path.read_text(encoding="utf-8", errors="replace"):
            seen.add(path)
    seen.update(cfg_gated_modules())
    return sorted(seen)


CFG_GATED_MOD = re.compile(
    r'#\[cfg\(target_os\s*=\s*"espidf"\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;'
)


def cfg_gated_modules() -> set[Path]:
    found: set[Path] = set()
    for path in ROOT.glob(CONTENT_SCAN_GLOB):
        if "target" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        base = path.parent if path.name in ("mod.rs", "lib.rs", "main.rs") else path.with_suffix("")
        for name in CFG_GATED_MOD.findall(text):
            for candidate in (base / f"{name}.rs", base / name / "mod.rs"):
                if candidate.exists():
                    found.add(candidate)
    return found


def collect() -> list[tuple[Path, str, int, int]]:
    results: list[tuple[Path, str, int, int]] = []
    for path in scanned_files():
        for name, line, count in scan_file(path):
            results.append((path.relative_to(ROOT), name, line, count))
    return results


def read_budget() -> int:
    if not BUDGET_FILE.exists():
        return 0
    try:
        return int(BUDGET_FILE.read_text().split()[0])
    except (ValueError, IndexError):
        return 0


def main() -> int:
    report_only = "--report" in sys.argv
    findings = collect()

    for rel, name, line, count in findings:
        print(f"  {rel}:{line}  fn {name}  ~{count} code lines (> {THRESHOLD})")

    budget = read_budget()
    total = len(findings)

    if report_only:
        print(f"verify_fn_length_esp: {total} finding(s), budget {budget} (report-only).")
        return 0

    if total > budget:
        print(
            f"verify_fn_length_esp: FAIL — {total} function(s) over {THRESHOLD} "
            f"code lines, budget is {budget}.\n"
            "Decompose the new one, or justify it in the PR and raise the budget "
            "deliberately (CLAUDE.md: budgets are meant to shrink).",
            file=sys.stderr,
        )
        return 1

    if total < budget:
        print(
            f"verify_fn_length_esp: FAIL — only {total} finding(s) but the budget "
            f"is {budget}. Lower scripts/fn_length_esp_budget.txt to {total}; the "
            "baseline must ratchet down as functions are decomposed.",
            file=sys.stderr,
        )
        return 1

    print(f"verify_fn_length_esp: OK (over_threshold={total} budget={budget})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
