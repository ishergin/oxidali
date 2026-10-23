#!/usr/bin/env python3
import argparse
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
from verify_imports_at_module_level import blank, masked

ALLOWLIST = "scripts/spawn_sites.txt"
SPAWN_HELPERS = "crates/dali2rust-bsp/src/esp_thread.rs"
OWN_HELPERS = frozenset({SPAWN_HELPERS, "tools/dali-gear-sim/src/spawn.rs"})
SCAN_GLOBS = ("crates/*/src/**/*.rs", "tools/dali-gear-sim/src/**/*.rs")
TEST_DIR = "tests"
MODULE_ROOTS = ("mod.rs", "lib.rs", "main.rs")

THROUGH_ESP_THREAD = ("boot", "runner", "occasion")
HOST_ONLY = "host"
OCCASION = "occasion"
MAX_OCCASION_SITES = 1
DYNAMIC = "<dynamic>"

SPAWNER = re.compile(
    r"\bpub\s+fn\s+(\w+)\s*(?:<[^>(]*>)?\s*\(\s*name\s*:\s*&'static\s+CStr"
)
CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?test\s*[,)]")
INNER_CFG_TEST = re.compile(r"#\s*!\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
ATTRIBUTE = re.compile(r"\s*#\s*\[")
MOD_DECLARATION = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
NAME_LITERAL = re.compile(r'c"([^"\\]*)"')
DIRECT_THREAD = re.compile(r"\bthread::(spawn|scope|Builder)\b")
DIRECT_THREAD_IMPORT = re.compile(r"\bthread::\{[^}]*?\b(spawn|scope|Builder)\b")


@dataclass(frozen=True)
class Site:
    path: str
    name: str
    line: int
    how: str
    host_only: bool

    @property
    def key(self):
        return (self.path, self.name)

    def classes(self):
        return (HOST_ONLY,) if self.host_only else THROUGH_ESP_THREAD


@dataclass(frozen=True)
class Entry:
    path: str
    name: str
    cls: str
    reason: str
    number: int


def close_of(code, start):
    depth = 0
    for i in range(start, len(code)):
        if code[i] in "([{":
            depth += 1
        elif code[i] in ")]}":
            depth -= 1
            if depth == 0:
                return i
    return len(code) - 1


def after_attributes(code, i):
    found = ATTRIBUTE.match(code, i)
    while found:
        i = close_of(code, found.end() - 1) + 1
        found = ATTRIBUTE.match(code, i)
    return i


def item_end(code, start):
    depth = 0
    for i in range(start, len(code)):
        ch = code[i]
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif depth == 0 and ch == ";":
            return i + 1
        elif depth == 0 and ch == "{":
            return close_of(code, i) + 1
    return len(code)


def drop_test_items(code):
    if INNER_CFG_TEST.search(code):
        return None, []
    spans, modules = [], []
    for found in CFG_TEST.finditer(code):
        attribute_end = close_of(code, code.index("[", found.start())) + 1
        start = after_attributes(code, attribute_end)
        end = item_end(code, start)
        declared = MOD_DECLARATION.match(code[start:end])
        modules.extend([declared.group(1)] if declared else [])
        spans.append((found.start(), end))
    for start, end in spans:
        code = code[:start] + blank(code[start:end]) + code[end:]
    return code, modules


def module_files(path, name):
    base = path.parent if path.name in MODULE_ROOTS else path.with_suffix("")
    files = {base / f"{name}.rs"}
    files.update((base / name).rglob("*.rs"))
    return {candidate for candidate in files if candidate.exists()}


def source_files(root):
    found = set()
    for pattern in SCAN_GLOBS:
        found.update(root.glob(pattern))
    return sorted(path for path in found if TEST_DIR not in path.relative_to(root).parts)


def analyse(root):
    production, test_only = {}, set()
    for path in source_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        code, modules = drop_test_items(masked(text))
        if code is not None:
            production[path] = (text, code)
        else:
            test_only.add(path)
        for name in modules:
            test_only.update(module_files(path, name))
    return {path: source for path, source in production.items() if path not in test_only}


def spawner_names(root):
    helper = root / SPAWN_HELPERS
    if not helper.exists():
        raise SystemExit(f"verify_spawn_sites: missing {SPAWN_HELPERS}")
    names = SPAWNER.findall(masked(helper.read_text(encoding="utf-8")))
    if not names:
        raise SystemExit(f"verify_spawn_sites: {SPAWN_HELPERS} declares no spawning function")
    return sorted(names, key=len, reverse=True)


def first_argument(code, text, open_paren):
    depth = 0
    for i in range(open_paren + 1, len(code)):
        if code[i] in "([{":
            depth += 1
        elif code[i] in ")]}" and depth == 0:
            return text[open_paren + 1 : i]
        elif code[i] in ")]}":
            depth -= 1
        elif code[i] == "," and depth == 0:
            return text[open_paren + 1 : i]
    return text[open_paren + 1 :]


def task_name(code, text, open_paren):
    literal = NAME_LITERAL.fullmatch(first_argument(code, text, open_paren).strip())
    return literal.group(1) if literal else DYNAMIC


def line_of(code, offset):
    return code.count("\n", 0, offset) + 1


def helper_sites(rel, text, code, names):
    call = re.compile(r"\b(" + "|".join(names) + r")\b\s*(?:::\s*<[^>]*>\s*)?\(")
    sites = []
    for found in call.finditer(code):
        if code[: found.start()].rstrip().endswith("fn"):
            continue
        name = task_name(code, text, found.end() - 1)
        sites.append(Site(rel, name, line_of(code, found.start()), found.group(1), False))
    return sites


def direct_sites(rel, code):
    sites = []
    for pattern in (DIRECT_THREAD, DIRECT_THREAD_IMPORT):
        for found in pattern.finditer(code):
            what = f"thread::{found.group(1)}"
            sites.append(Site(rel, what, line_of(code, found.start()), what, True))
    return sites


def collect(root):
    names = spawner_names(root)
    sites = []
    for path, (text, code) in analyse(root).items():
        rel = path.relative_to(root).as_posix()
        if rel == SPAWN_HELPERS:
            continue
        sites.extend(helper_sites(rel, text, code, names))
        if rel not in OWN_HELPERS:
            sites.extend(direct_sites(rel, code))
    return sorted(sites, key=lambda site: (site.path, site.line))


def read_allowlist(root):
    path = root / ALLOWLIST
    if not path.exists():
        raise SystemExit(f"verify_spawn_sites: missing {ALLOWLIST}")
    entries, problems = [], []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 3)
        if len(parts) < 4:
            problems.append(f"{ALLOWLIST}:{number}: want `<file> <task-name> <class> <reason>`, a reason included")
        elif parts[2] not in THROUGH_ESP_THREAD + (HOST_ONLY,):
            problems.append(f"{ALLOWLIST}:{number}: class `{parts[2]}` is none of {', '.join(THROUGH_ESP_THREAD + (HOST_ONLY,))}")
        else:
            entries.append(Entry(parts[0], parts[1], parts[2], parts[3], number))
    return entries, problems


def reconcile(sites, entries):
    pool = defaultdict(list)
    for entry in entries:
        pool[(entry.path, entry.name)].append(entry)
    unlisted, misclassed = [], []
    for site in sites:
        bucket = pool.get(site.key)
        if not bucket:
            unlisted.append(site)
            continue
        entry = bucket.pop(0)
        if entry.cls not in site.classes():
            misclassed.append((site, entry))
    dead = [entry for bucket in pool.values() for entry in bucket]
    return unlisted, misclassed, sorted(dead, key=lambda entry: entry.number)


def describe_problems(unlisted, misclassed, dead, entries):
    problems = []
    for site in unlisted:
        problems.append(
            f"{site.path}:{site.line}: `{site.how}` starts a thread named {site.name} that {ALLOWLIST} does not list; "
            "recurring work belongs on a StackRunner or RunnerPool, a boot-time worker or a host-only thread gets one line"
        )
    for site, entry in misclassed:
        problems.append(
            f"{ALLOWLIST}:{entry.number}: {site.path} {site.name} is class `{entry.cls}` but the call is "
            f"`{site.how}`; it takes one of {', '.join(site.classes())}"
        )
    for entry in dead:
        problems.append(f"{ALLOWLIST}:{entry.number}: {entry.path} {entry.name} matches no call site; drop the line")
    occasions = [entry for entry in entries if entry.cls == OCCASION]
    if len(occasions) > MAX_OCCASION_SITES:
        problems.append(
            f"{ALLOWLIST}: {len(occasions)} `{OCCASION}` lines against {MAX_OCCASION_SITES}; a thread per occasion "
            "leaks internal SRAM on ESP-IDF, so recurring work runs on a StackRunner or RunnerPool"
        )
    return problems


def summary(sites, entries):
    by_class = ", ".join(
        f"{sum(1 for entry in entries if entry.cls == cls)} {cls}" for cls in THROUGH_ESP_THREAD + (HOST_ONLY,)
    )
    return f"{len(sites)} spawn sites in {len({site.path for site in sites})} files: {by_class}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=SCRIPTS.parent)
    root = parser.parse_args().root.resolve()
    sites = collect(root)
    entries, problems = read_allowlist(root)
    unlisted, misclassed, dead = reconcile(sites, entries)
    problems.extend(describe_problems(unlisted, misclassed, dead, entries))
    if problems:
        print("\n".join(problems), file=sys.stderr)
        print(f"verify_spawn_sites: FAIL ({len(problems)} problem(s))", file=sys.stderr)
        return 1
    print(f"verify_spawn_sites: OK ({summary(sites, entries)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
