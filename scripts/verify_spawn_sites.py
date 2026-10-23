#!/usr/bin/env python3
import argparse
import re
import sys
import tomllib
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
from verify_imports_at_module_level import blank, masked

ALLOWLIST = "scripts/spawn_sites.txt"
ESP_HELPER = "crates/dali2rust-bsp/src/esp_thread.rs"
GEAR_HELPER = "tools/dali-gear-sim/src/spawn.rs"
GEAR_MANIFEST = "tools/dali-gear-sim/Cargo.toml"
MOCK_CLIENT = "crates/dali2rust-mqtt-runtime/src/client/mock.rs"
SPAWN_HELPERS = (ESP_HELPER, GEAR_HELPER)
STACK_RUNNER = "crates/dali2rust-bsp/src/stack_runner.rs"
FIRMWARE_CRATE = "dali2rust-firmware"
SCAN_GLOBS = ("crates/*/src/**/*.rs", "tools/dali-gear-sim/src/**/*.rs")
TEST_DIR = "tests"
MODULE_ROOTS = ("mod.rs", "lib.rs", "main.rs")
CRATE_ROOTS = ("lib.rs", "main.rs")

BOOT, RUNNER, OCCASION, HOST, HOST_LINKED = "boot", "runner", "occasion", "host", "host-linked"
THROUGH_ESP_THREAD = (BOOT, RUNNER, OCCASION)
CLASSES = THROUGH_ESP_THREAD + (HOST, HOST_LINKED)
OCCASION_CAP = 1
RELATIVE_ROOTS = ("super", "crate", "self")
DYNAMIC = "<dynamic>"
ESPIDF = '"espidf"'
THREAD_ITEMS = ("spawn", "scope", "Builder")

SPAWNER = re.compile(
    r"\bpub\s+fn\s+(\w+)\s*(?:<[^>(]*>)?\s*\(\s*name\s*:\s*&'static\s+CStr"
)
CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?test\s*[,)]")
INNER_CFG_TEST = re.compile(r"#\s*!\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
CFG_HOST = re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?not\s*\(\s*target_os\s*=\s*\)")
INNER_CFG_HOST = re.compile(r"#\s*!\s*\[\s*cfg\s*\(\s*not\s*\(\s*target_os\s*=\s*\)\s*\)\s*\]")
ATTRIBUTE = re.compile(r"\s*#\s*\[")
ITEM_HEAD = re.compile(r"(?:pub\b(?:\s*\([^)]*\))?\s*)?(\w+)")
ITEM_WORDS = frozenset(
    "fn mod impl use const static struct enum trait type union unsafe extern async default macro_rules".split()
)
MOD_PREFIX = r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
MOD_DECLARATION = re.compile(r"\s*" + MOD_PREFIX)
ALL_MODULES = re.compile(r"(?:^|(?<=[;{}\]]))\s*" + MOD_PREFIX)
USE_ITEM = re.compile(r"(?:^|(?<=[;{}\]]))\s*(pub\b(?:\s*\([^)]*\))?\s*)?use\s+([^;]*);")
USE_TOKENS = re.compile(r"::|[{},*]|\w+")
GLOB_SUPER = re.compile(r"\buse\s+super::(?:\*|\{[^}]*\*)")
MACRO_RULES = re.compile(r"\bmacro_rules\s*!\s*(\w+)\s*(?=[{(\[])")
NAME_LITERAL = re.compile(r'c"([^"\\]*)"')
CALL = re.compile(r"\s*(?:::\s*<[^>]*>\s*)?\(")
QUALIFIED = re.compile(r"\bthread::(spawn|scope|Builder)\b")
FN_KEYWORD = re.compile(r"\bfn\s+$")
BARE_PREFIX = r"(?<![\w.])(?<!::)"
BARE_FORMS = {
    "spawn": r"spawn\s*(?:::\s*<[^>]*>\s*)?\(",
    "scope": r"scope\s*(?:::\s*<[^>]*>\s*)?\(",
    "Builder": r"Builder\s*::",
}


@dataclass(frozen=True)
class Site:
    path: str
    name: str
    line: int
    how: str
    direct: bool
    host_only: bool
    macro: str

    @property
    def key(self):
        return (self.path, self.name)

    def classes(self):
        if not self.direct:
            return THROUGH_ESP_THREAD
        return (HOST,) if self.host_only else (HOST_LINKED,)

    def where(self):
        if not self.direct:
            return f"the call is `{self.how}`, a spawn through a helper"
        if self.host_only:
            return "the call is a std thread in code compiled off ESP-IDF only"
        return (
            "the call is a std thread in code the device compiles; `host` needs it under "
            '#[cfg(not(target_os = "espidf"))], in a module declared with that cfg, or in a crate the firmware does not link'
        )

    def refusal(self):
        if self.macro:
            return (
                f"{self.path}:{self.line}: `{self.how}` sits in macro_rules! {self.macro}; a macro hides how many "
                "times it starts a thread, so write the call out"
            )
        if self.name == DYNAMIC and self.path != STACK_RUNNER:
            return (
                f'{self.path}:{self.line}: `{self.how}` has no literal c"..." task name at the call; '
                "name the task there, or run recurring work on a StackRunner or RunnerPool"
            )
        return None


@dataclass(frozen=True)
class Entry:
    path: str
    name: str
    cls: str
    reason: str
    number: int


@dataclass(frozen=True)
class Imports:
    bare: frozenset
    spans: tuple
    refusals: tuple


@dataclass(frozen=True)
class HostScope:
    files: frozenset
    spans: dict
    off_device: frozenset

    def holds(self, path, offset):
        if path in self.files or any(parent in self.off_device for parent in path.parents):
            return True
        return any(start <= offset < end for start, end in self.spans.get(path, ()))


@dataclass(frozen=True)
class Scan:
    names: list
    production: dict
    imports: dict
    host: HostScope


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
    return len(code) - len(code[i:].lstrip())


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


def starts_item(code, start):
    head = ITEM_HEAD.match(code, start)
    return code.startswith("{", start) or bool(head and head.group(1) in ITEM_WORDS)


def all_modules(code):
    return [found.group(1) for found in ALL_MODULES.finditer(code)]


def attribute_items(code, attributes):
    spans, modules = [], []
    for found in attributes:
        attribute_end = close_of(code, code.index("[", found.start())) + 1
        start = after_attributes(code, attribute_end)
        if not starts_item(code, start):
            continue
        end = item_end(code, start)
        declared = MOD_DECLARATION.match(code[start:end])
        modules.extend([declared.group(1)] if declared else [])
        spans.append((found.start(), end))
    return spans, modules


def drop_test_items(code):
    if INNER_CFG_TEST.search(code):
        return None, all_modules(code)
    spans, modules = attribute_items(code, CFG_TEST.finditer(code))
    for start, end in spans:
        code = code[:start] + blank(code[start:end]) + code[end:]
    return code, modules


def module_files(path, name):
    base = path.parent if path.name in MODULE_ROOTS else path.with_suffix("")
    files = {base / f"{name}.rs"}
    files.update((base / name).rglob("*.rs"))
    return {candidate for candidate in files if candidate.exists()}


def parent_file(path, production):
    if path.name in CRATE_ROOTS:
        return None
    home = path.parent.parent if path.name == "mod.rs" else path.parent
    candidates = [home / root for root in MODULE_ROOTS] + [home.with_suffix(".rs")]
    return next((c for c in candidates if c in production and c != path), None)


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


def names_espidf(text, found):
    return ESPIDF in text[found.start() : found.end()]


def file_host_scope(text, code):
    if any(names_espidf(text, found) for found in INNER_CFG_HOST.finditer(code)):
        return None, all_modules(code)
    attributes = [found for found in CFG_HOST.finditer(code) if names_espidf(text, found)]
    return attribute_items(code, attributes)


def read_manifest(root, rel):
    path = root / rel
    if not path.exists():
        raise SystemExit(f"verify_spawn_sites: missing {rel}")
    return tomllib.loads(path.read_text(encoding="utf-8"))


def workspace_paths(manifest):
    table = manifest.get("workspace", {}).get("dependencies", {})
    return {name: spec["path"] for name, spec in table.items() if isinstance(spec, dict) and "path" in spec}


def linked_dependencies(manifest, inherited):
    tables = [manifest.get("dependencies", {})]
    tables.extend(target.get("dependencies", {}) for target in manifest.get("target", {}).values())
    found = set()
    for table in tables:
        for name, spec in table.items():
            spec = spec if isinstance(spec, dict) else {}
            where = spec.get("path") or (inherited.get(name) if spec.get("workspace") else None)
            if where:
                found.add(Path(where).name)
    return found


def device_closure(root):
    inherited = workspace_paths(read_manifest(root, "Cargo.toml"))
    gear = read_manifest(root, GEAR_MANIFEST)
    pending = [FIRMWARE_CRATE, *linked_dependencies(gear, workspace_paths(gear))]
    seen = set()
    while pending:
        crate = pending.pop()
        if crate not in seen and (root / "crates" / crate).is_dir():
            seen.add(crate)
            pending.extend(linked_dependencies(read_manifest(root, f"crates/{crate}/Cargo.toml"), inherited))
    return seen


def off_device_crates(root):
    linked = device_closure(root)
    return frozenset(d for d in (root / "crates").iterdir() if d.is_dir() and d.name not in linked)


def host_scope(root, production):
    files, spans = set(), {}
    for path, (text, code) in production.items():
        found, modules = file_host_scope(text, code)
        if found is None:
            files.add(path)
        else:
            spans[path] = found
        for name in modules:
            files.update(module_files(path, name))
    return HostScope(frozenset(files), spans, off_device_crates(root))


def spawner_names(root):
    names = []
    for helper in SPAWN_HELPERS:
        path = root / helper
        if not path.exists():
            raise SystemExit(f"verify_spawn_sites: missing {helper}")
        found = SPAWNER.findall(masked(path.read_text(encoding="utf-8")))
        if not found:
            raise SystemExit(f"verify_spawn_sites: {helper} declares no spawning function")
        names.extend(found)
    return sorted(set(names), key=len, reverse=True)


def parse_group(tokens, i, path):
    leaves = []
    while i < len(tokens) and tokens[i] != "}":
        found, i = parse_tree(tokens, i, path)
        leaves.extend(found)
        i += 1 if i < len(tokens) and tokens[i] == "," else 0
    return leaves, i + 1


def parse_tree(tokens, i, prefix):
    path = list(prefix)
    while i < len(tokens) and tokens[i] not in (",", "}"):
        if tokens[i] == "{":
            return parse_group(tokens, i + 1, tuple(path))
        if tokens[i] == "as":
            return [(tuple(path), tokens[i + 1] if i + 1 < len(tokens) else "_")], i + 2
        path.extend([] if tokens[i] == "::" else [tokens[i]])
        i += 1
    return [(tuple(path), None)], i


def use_leaves(tree):
    leaves, _ = parse_tree(USE_TOKENS.findall(tree), 0, ())
    return [(path[:-1] if path[-1] == "self" else path, alias) for path, alias in leaves if path]


def thread_item(path):
    if len(path) < 2:
        return False
    if path[-2] == "thread":
        return path[-1] in THREAD_ITEMS or path[-1] == "*"
    return path[0] in RELATIVE_ROOTS and path[-1] in THREAD_ITEMS


def judge_leaf(path, alias, public, names):
    tail = path[-1]
    item = thread_item(path)
    if alias and (item or tail == "thread" or tail in names):
        return set(), f"renames `{'::'.join(path)}`"
    if public and item:
        return set(), f"re-exports `{'::'.join(path)}`"
    return (set(THREAD_ITEMS) if tail == "*" else {tail}) if item else set(), None


def read_imports(code, names):
    bare, spans, refusals = set(), [], []
    for found in USE_ITEM.finditer(code):
        spans.append(found.span())
        for path, alias in use_leaves(found.group(2)):
            added, refused = judge_leaf(path, alias, bool(found.group(1)), names)
            bare |= added
            refusals.extend([(found.start(2), refused)] if refused else [])
    return Imports(frozenset(bare), tuple(spans), tuple(refusals))


def effective_bare(path, scan):
    own = scan.imports[path].bare
    glob = GLOB_SUPER.search(scan.production[path][1])
    parent = parent_file(path, scan.production) if glob else None
    return own | effective_bare(parent, scan) if parent else own


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


def spawner_hits(code, text, names):
    pattern = re.compile(r"(?<!\w)(" + "|".join(names) + r")\b")
    hits = []
    for found in pattern.finditer(code):
        if FN_KEYWORD.search(code[max(0, found.start() - 12) : found.start()]):
            continue
        call = CALL.match(code, found.end())
        name = task_name(code, text, call.end() - 1) if call else DYNAMIC
        hits.append((found.start(), name, found.group(1), False))
    return hits


def thread_hits(code, bare):
    hits = [(found.start(), f"thread::{found.group(1)}") for found in QUALIFIED.finditer(code)]
    for name in sorted(bare):
        hits.extend((found.start(), f"thread::{name}") for found in re.finditer(BARE_PREFIX + BARE_FORMS[name], code))
    return [(offset, what, what, True) for offset, what in hits]


def file_hits(code, text, names, import_spans, bare):
    body = code
    for start, end in import_spans:
        body = body[:start] + blank(body[start:end]) + body[end:]
    return spawner_hits(body, text, names) + thread_hits(body, bare)


def macro_spans(code):
    return [(f.start(), close_of(code, f.end()) + 1, f.group(1)) for f in MACRO_RULES.finditer(code)]


def macro_at(macros, offset):
    return next((name for start, end, name in macros if start <= offset < end), "")


def file_sites(scan, path, rel):
    text, code = scan.production[path]
    macros = macro_spans(code)
    hits = file_hits(code, text, scan.names, scan.imports[path].spans, effective_bare(path, scan))
    return [
        Site(rel, name, line_of(code, offset), how, direct, scan.host.holds(path, offset), macro_at(macros, offset))
        for offset, name, how, direct in hits
    ]


def import_refusals(rel, code, imports):
    return [
        f"{rel}:{line_of(code, offset)}: `use` {reason}; a renamed or re-exported thread start hides its call "
        "sites, so use it under its own name"
        for offset, reason in imports.refusals
    ]


def collect(root):
    names, production = spawner_names(root), analyse(root)
    imports = {path: read_imports(code, names) for path, (_, code) in production.items()}
    scan = Scan(names, production, imports, host_scope(root, production))
    sites, refusals = [], []
    for path in production:
        rel = path.relative_to(root).as_posix()
        if rel not in SPAWN_HELPERS:
            sites.extend(file_sites(scan, path, rel))
            refusals.extend(import_refusals(rel, production[path][1], imports[path]))
    return sorted(sites, key=lambda site: (site.path, site.line)), refusals


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
        elif parts[2] not in CLASSES:
            problems.append(f"{ALLOWLIST}:{number}: class `{parts[2]}` is none of {', '.join(CLASSES)}")
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


def reconcile_problems(unlisted, misclassed, dead):
    problems = [
        f"{site.path}:{site.line}: `{site.how}` starts a thread named {site.name} that {ALLOWLIST} does not list; "
        "recurring work belongs on a StackRunner or RunnerPool, a boot-time worker or a host-only thread gets one line"
        for site in unlisted
    ]
    problems.extend(
        f"{ALLOWLIST}:{entry.number}: {site.path} {site.name} is class `{entry.cls}` but {site.where()}; "
        f"it takes {' or '.join(site.classes())}"
        for site, entry in misclassed
    )
    problems.extend(
        f"{ALLOWLIST}:{entry.number}: {entry.path} {entry.name} matches no call site; drop the line" for entry in dead
    )
    return problems


def runner_problems(entries):
    problems = []
    for entry in entries:
        runner_place = (entry.path, entry.name) == (STACK_RUNNER, DYNAMIC)
        if (entry.cls == RUNNER) != runner_place or (entry.name == DYNAMIC) != runner_place:
            problems.append(
                f"{ALLOWLIST}:{entry.number}: {DYNAMIC} and class `{RUNNER}` belong to the one line of "
                f"{STACK_RUNNER}; every other line names a literal task"
            )
    return problems


def host_linked_problems(entries):
    return [
        f"{ALLOWLIST}:{entry.number}: class `{HOST_LINKED}` is for {MOCK_CLIENT} alone, the one host-only mock the "
        'firmware image compiles; gate other host threads with #[cfg(not(target_os = "espidf"))]'
        for entry in entries
        if entry.cls == HOST_LINKED and entry.path != MOCK_CLIENT
    ]


def occasion_problems(entries):
    count = sum(1 for entry in entries if entry.cls == OCCASION)
    if count <= OCCASION_CAP:
        return []
    return [
        f"{ALLOWLIST}: {count} `{OCCASION}` lines against {OCCASION_CAP}; a thread per occasion leaks "
        "internal SRAM on ESP-IDF, so recurring work runs on a StackRunner or RunnerPool"
    ]


def entry_problems(entries):
    return runner_problems(entries) + host_linked_problems(entries) + occasion_problems(entries)


def summary(sites, entries):
    by_class = ", ".join(f"{sum(1 for entry in entries if entry.cls == cls)} {cls}" for cls in CLASSES)
    return f"{len(sites)} spawn sites in {len({site.path for site in sites})} files: {by_class}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=SCRIPTS.parent)
    root = parser.parse_args().root.resolve()
    sites, problems = collect(root)
    entries, listing = read_allowlist(root)
    refused = [site for site in sites if site.refusal()]
    problems.extend(site.refusal() for site in refused)
    problems.extend(listing)
    kept = [site for site in sites if site not in refused]
    problems.extend(reconcile_problems(*reconcile(kept, entries)))
    problems.extend(entry_problems(entries))
    if problems:
        print("\n".join(problems), file=sys.stderr)
        print(f"verify_spawn_sites: FAIL ({len(problems)} problem(s))", file=sys.stderr)
        return 1
    print(f"verify_spawn_sites: OK ({summary(sites, entries)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
