#!/usr/bin/env python3
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "documentation/product-design/issue-ids-registry.md"
KNOWN = ROOT / "documentation/product-design/known-issues.md"
STRATEGY = ROOT / "tools/hil/STRATEGY.md"
WATCHLIST_HEADING = "## 7. Watchlist"
HOME_KNOWN, HOME_BENCH, CLOSED = "known-issues", "strategy", "closed"
KNOWN_STATUSES = ("открыт", "ждёт стенда", "обход")
BENCH_STATUS = "стенд"
CLOSED_STATUS = "закрыт"
SKIP_DIRS = {".git", "target", "node_modules", ".venv", "runs", "vendor", "dist"}
SCAN_SUFFIXES = {".rs", ".py", ".md", ".ts", ".tsx", ".txt", ".toml", ".sh", ".feature"}
ROW = re.compile(r"^\| ISSUE-(\d+) \| ([^|]+?) \| ([^|]+?) \|$")

errors: list[str] = []


def fail(msg: str) -> None:
    errors.append(msg)


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def classify(status: str) -> str | None:
    if status.startswith(CLOSED_STATUS):
        return CLOSED
    if status == BENCH_STATUS:
        return HOME_BENCH
    if status in KNOWN_STATUSES:
        return HOME_KNOWN
    return None


def parse_registry() -> dict[int, str]:
    homes: dict[int, str] = {}
    for line in read(REGISTRY).splitlines():
        if not line.startswith("| ISSUE-"):
            continue
        m = ROW.match(line)
        if not m:
            fail(f"registry: malformed row: {line[:80]}")
            continue
        num, status = int(m.group(1)), m.group(2).strip()
        home = classify(status)
        if home is None:
            fail(f"registry: ISSUE-{num} has unknown status '{status}'")
            continue
        if home == CLOSED and not re.search(r"\(.+\)", status):
            fail(f"registry: closed ISSUE-{num} names no commit or ADR")
        if num in homes:
            fail(f"registry: ISSUE-{num} claimed twice")
        homes[num] = home
    return homes


def parse_records() -> tuple[set[int], set[int]]:
    known = {int(m) for m in re.findall(r"^###? ISSUE-(\d+)\b", read(KNOWN), re.M)}
    body = read(STRATEGY).split(WATCHLIST_HEADING, 1)
    if len(body) != 2:
        fail(f"STRATEGY.md: section '{WATCHLIST_HEADING}' not found")
        return known, set()
    watchlist = body[1].split("\n## ", 1)[0]
    bench = {int(m) for m in re.findall(r"^\| ISSUE-(\d+) \|", watchlist, re.M)}
    return known, bench


def scan_citations() -> dict[int, str]:
    cited: dict[int, str] = {}
    for path in ROOT.rglob("*"):
        if not path.is_file() or path.suffix not in SCAN_SUFFIXES:
            continue
        rel = path.relative_to(ROOT)
        if any(part in SKIP_DIRS for part in rel.parts):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for i, line in enumerate(text.splitlines(), 1):
            for m in re.finditer(r"\bISSUE-(\d+)\b", line):
                cited.setdefault(int(m.group(1)), f"{rel}:{i}")
    return cited


def check_homes(homes: dict[int, str], known: set[int], bench: set[int]) -> None:
    for num in sorted(known - set(homes)):
        fail(f"ISSUE-{num}: record in known-issues.md, not allocated in the registry")
    for num in sorted(bench - set(homes)):
        fail(f"ISSUE-{num}: watchlist row in STRATEGY.md, not allocated in the registry")
    for num, home in sorted(homes.items()):
        if home == HOME_KNOWN and num not in known:
            fail(f"ISSUE-{num}: registry says known-issues.md, but no 'ISSUE-{num}' heading is there")
        if home == HOME_BENCH and num not in bench:
            fail(f"ISSUE-{num}: registry says STRATEGY.md §7, but no watchlist row is there")
        if home != HOME_KNOWN and num in known:
            fail(f"ISSUE-{num}: known-issues.md carries a record, but the registry says '{home}'")
        if home == CLOSED and num in bench:
            fail(f"ISSUE-{num}: closed, but STRATEGY.md §7 still carries a row")


def main() -> int:
    for path in (REGISTRY, KNOWN, STRATEGY):
        if not path.exists():
            print(f"FAIL: missing {path.relative_to(ROOT)}")
            return 1
    homes = parse_registry()
    if not homes:
        print("FAIL: registry parsed no ids")
        return 1
    known, bench = parse_records()
    check_homes(homes, known, bench)
    for num, where in sorted(scan_citations().items()):
        if num not in homes:
            fail(f"ISSUE-{num}: cited but never allocated (e.g. {where})")
    gaps = [n for n in range(1, max(homes) + 1) if n not in homes]
    if gaps:
        fail(f"registry: unallocated gaps in the number line: {gaps}")
    if errors:
        print("FAIL: issue-id integrity")
        for e in errors:
            print(f"  - {e}")
        return 1
    counts = {k: sum(v == k for v in homes.values()) for k in (HOME_KNOWN, HOME_BENCH, CLOSED)}
    print(
        f"OK: {len(homes)} issue ids ({counts[HOME_KNOWN]} in known-issues, "
        f"{counts[HOME_BENCH]} on the bench watchlist, {counts[CLOSED]} closed)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
