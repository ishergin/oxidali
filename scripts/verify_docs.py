#!/usr/bin/env python3
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CAPS_FILE = ROOT / "scripts" / "doc_size_caps.txt"
DEFAULT_CAP_KB = 40
SHINGLE_WORDS = 40
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
ISO_DATE = re.compile(r"\b20\d\d-\d\d-\d\d\b")
FENCE = re.compile(r"^\s*(```|~~~)")
DATE_SCOPE = ("documentation/", "CLAUDE.md", "README.md", "tools/hil/README.md", "tools/hil/STRATEGY.md")
DATE_FREE_FILES = (
    "documentation/product-design/known-issues.md",
    "documentation/product-design/issue-ids-registry.md",
)
EXCLUDED = ("web/design-system/", "crates/dali2rust-firmware/assets/")
SHINGLE_EXCLUDED = ("AGENTS.md",)


def tracked_markdown():
    res = subprocess.run(["git", "ls-files", "*.md", "*.mdc"], cwd=ROOT, capture_output=True, text=True, check=True)
    files = [f for f in res.stdout.splitlines() if f and not f.startswith(EXCLUDED)]
    return [f for f in files if (ROOT / f).exists()]


def untracked_markdown():
    res = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "*.md"], cwd=ROOT, capture_output=True, text=True
    )
    return [f for f in res.stdout.splitlines() if f and not f.startswith(EXCLUDED)]


def prose_lines(text):
    in_fence = False
    for number, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if not in_fence:
            yield number, line


def check_links(rel, text, errors):
    base = (ROOT / rel).parent
    for number, line in prose_lines(text):
        for m in LINK.finditer(line):
            target = m.group(1)
            if target.startswith(("http://", "https://", "mailto:", "#")):
                continue
            path = target.split("#", 1)[0]
            if path and not (base / path).exists():
                errors.append(f"{rel}:{number}: broken link -> {target}")


def read_caps():
    caps = {}
    if CAPS_FILE.exists():
        for line in CAPS_FILE.read_text().splitlines():
            parts = line.split(None, 2)
            if len(parts) >= 3 and not line.startswith("#"):
                caps[parts[0]] = int(parts[1])
    return caps


def check_size(rel, text, caps, errors):
    size_kb = len(text.encode("utf-8")) / 1024
    cap = caps.get(rel, DEFAULT_CAP_KB)
    if size_kb > cap:
        errors.append(f"{rel}: {size_kb:.1f} KB exceeds the {cap} KB cap")


def date_allowed(rel, line):
    if rel in DATE_FREE_FILES:
        return True
    if "/decisions/" in rel and line.startswith("Date:"):
        return True
    return rel == "tools/hil/STRATEGY.md" and line.startswith("| ISSUE-")


def check_dates(rel, text, errors):
    if not rel.startswith(DATE_SCOPE):
        return
    for number, line in prose_lines(text):
        if ISO_DATE.search(line) and not date_allowed(rel, line):
            errors.append(f"{rel}:{number}: dated text ({ISO_DATE.search(line).group(0)}) — state what is, not when")


def words_of(text):
    out = []
    for number, line in prose_lines(text):
        cleaned = re.sub(r"\]\([^)]*\)", "]", line)
        for word in re.findall(r"[\w§.%-]+", cleaned.lower()):
            out.append((word, number))
    return out


def check_duplicates(texts, errors):
    seen = {}
    hits = defaultdict(set)
    for rel, text in texts.items():
        if rel.startswith(SHINGLE_EXCLUDED):
            continue
        words = words_of(text)
        last_end = {}
        for i in range(len(words) - SHINGLE_WORDS + 1):
            key = " ".join(w for w, _ in words[i:i + SHINGLE_WORDS])
            origin = seen.get(key)
            if origin is None:
                seen[key] = (rel, i, words[i][1])
                continue
            other, j, line = origin
            if other == rel and i - j < SHINGLE_WORDS:
                continue
            pair = (other, line, rel)
            if last_end.get(pair, -1) >= i:
                continue
            hits[(other, rel)].add((line, words[i][1]))
            last_end[pair] = i + SHINGLE_WORDS
    for (a, b), spots in sorted(hits.items()):
        first = sorted(spots)[0]
        errors.append(f"{b}:{first[1]}: repeats {a}:{first[0]} ({len(spots)} passage(s) of ≥{SHINGLE_WORDS} words)")


def main():
    errors = []
    caps = read_caps()
    files = tracked_markdown() + untracked_markdown()
    texts = {rel: (ROOT / rel).read_text(encoding="utf-8") for rel in files}
    for rel, text in texts.items():
        check_links(rel, text, errors)
        check_size(rel, text, caps, errors)
        check_dates(rel, text, errors)
    check_duplicates(texts, errors)
    stale_caps = [p for p in caps if p not in texts]
    for p in stale_caps:
        errors.append(f"scripts/doc_size_caps.txt: {p} no longer exists — remove the line")
    if errors:
        for e in errors:
            print(e, file=sys.stderr)
        print(f"verify_docs: {len(errors)} problem(s) in {len(texts)} documents", file=sys.stderr)
        return 1
    print(f"verify_docs: OK ({len(texts)} documents: links resolve, sizes within caps, no dated text, no repeats)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
