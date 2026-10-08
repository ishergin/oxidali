#!/usr/bin/env python3
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from host_crates import host_crates

EXTRA_TREES = ("crates/dali2rust-firmware/", "tests/dali2rust-bdd/", "tools/dali-gear-sim/")
FN_HEAD = re.compile(r"\bfn\s+[A-Za-z_]\w*")
MOD_HEAD = re.compile(r"\bmod\s+[A-Za-z_]\w*\s*$")
USE_STATEMENT = re.compile(r"(?:^|(?<=[;{}\]]))(\s*)((?:pub(?:\([^)]*\))?\s+)?use\s)")
STRING_PREFIX = re.compile(r'(?:b?r(#*)")|(?:b?")')
CHAR_LITERAL = re.compile(r"b?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]{1,6}\}|.)|[^\\'\n])'")


def scanned_files():
    res = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "*.rs"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    trees = tuple(f"crates/{name}/" for name in host_crates()) + EXTRA_TREES
    return sorted({rel for rel in res.stdout.splitlines() if rel.startswith(trees) and (ROOT / rel).exists()})


def blank(text):
    return re.sub(r"[^\n]", " ", text)


def skip_block_comment(text, start):
    depth, i = 1, start + 2
    while i < len(text) and depth:
        if text.startswith("/*", i):
            depth, i = depth + 1, i + 2
        elif text.startswith("*/", i):
            depth, i = depth - 1, i + 2
        else:
            i += 1
    return i


def skip_string(text, start, match):
    hashes = match.group(1)
    i = match.end()
    if hashes is not None:
        closing = '"' + hashes
        end = text.find(closing, i)
        return len(text) if end < 0 else end + len(closing)
    while i < len(text):
        if text[i] == "\\":
            i += 2
        elif text[i] == '"':
            return i + 1
        else:
            i += 1
    return i


def literal_end(text, i):
    if text.startswith("//", i):
        end = text.find("\n", i)
        return len(text) if end < 0 else end
    if text.startswith("/*", i):
        return skip_block_comment(text, i)
    if i and (text[i - 1].isalnum() or text[i - 1] == "_"):
        return None
    found = STRING_PREFIX.match(text, i)
    if found:
        return skip_string(text, i, found)
    found = CHAR_LITERAL.match(text, i)
    return found.end() if found else None


def masked(text):
    out, i, plain = [], 0, 0
    while i < len(text):
        end = literal_end(text, i) if text[i] in "/\"'br" else None
        if end is None:
            i += 1
            continue
        out.append(text[plain:i])
        out.append(blank(text[i:end]))
        i = plain = end
    out.append(text[plain:])
    return "".join(out)


def scope_kinds(code, probes):
    kinds, stack, pending, depth = {}, [], None, 0
    heads = {m.end() for m in FN_HEAD.finditer(code)}
    for i, ch in enumerate(code):
        if i in probes:
            kinds[i] = tuple(stack)
        if i in heads:
            pending = depth
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == ";" and pending == depth:
            pending = None
        elif ch == "{":
            if pending == depth:
                stack.append("fn")
                pending = None
            else:
                stack.append("mod" if MOD_HEAD.search(code, max(0, i - 80), i) else "block")
        elif ch == "}" and stack:
            stack.pop()
    return kinds


def in_fn_body(stack):
    for kind in reversed(stack):
        if kind == "mod":
            return False
        if kind == "fn":
            return True
    return False


def violations(rel):
    text = (ROOT / rel).read_text(encoding="utf-8")
    code = masked(text)
    starts = [match.start(2) for match in USE_STATEMENT.finditer(code)]
    kinds = scope_kinds(code, set(starts))
    found = []
    for start in starts:
        if in_fn_body(kinds[start]):
            number = code.count("\n", 0, start) + 1
            found.append((rel, number, text.splitlines()[number - 1].strip()))
    return found


def main():
    findings = [hit for rel in scanned_files() for hit in violations(rel)]
    for rel, number, line in findings:
        print(f"{rel}:{number}: {line}", file=sys.stderr)
    if findings:
        print(
            f"verify_imports_at_module_level: {len(findings)} `use` inside a function body; "
            "move each to the top of its module (a test module's own top for test code)",
            file=sys.stderr,
        )
        return 1
    print("verify_imports_at_module_level: OK (every `use` is at module level)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
