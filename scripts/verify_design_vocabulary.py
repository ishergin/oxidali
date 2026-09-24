#!/usr/bin/env python3
import glob
import re
import sys


def classes_in(css: str) -> set[str]:
    return set(re.findall(r"\.([A-Za-z][\w-]*)", css))


def markup_classes(text: str) -> set[str]:
    text = re.sub(r"<style[^>]*>.*?</style>", "", text, flags=re.S)
    text = re.sub(r"\{/\*.*?\*/\}", "", text, flags=re.S)
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    out: set[str] = set()
    for m in re.finditer(r'class="([^"{}]*)"', text):
        out.update(c for c in m.group(1).split() if c)
    return out


def root_block(text: str) -> str | None:
    m = re.search(r":root\s*\{.*?\n\}", text, re.S)
    return re.sub(r"\s+", " ", m.group(0)).strip() if m else None


fail = []

canon_src = open("web/design-system/tokens.css", encoding="utf-8").read()
canon = root_block(canon_src)
cards = sorted(glob.glob("web/design-system/**/*.html", recursive=True))
drifted = [p for p in cards if root_block(open(p, encoding="utf-8").read()) != canon]
if drifted:
    fail.append(
        "A. cards whose :root differs from tokens.css (it claims to be inlined verbatim):\n"
        + "\n".join(f"     {p}" for p in drifted)
    )

unstyled_cards = []
ds_vocabulary: set[str] = set()
for p in cards:
    text = open(p, encoding="utf-8").read()
    defined = set()
    for css in re.findall(r"<style[^>]*>(.*?)</style>", text, re.S):
        defined |= classes_in(css)
    ds_vocabulary |= defined
    missing = markup_classes(text) - defined
    if missing:
        unstyled_cards.append(f"     {p}: {', '.join(sorted(missing))}")
if unstyled_cards:
    fail.append("B. cards using a class no rule defines:\n" + "\n".join(unstyled_cards))

allowed = set()
for line in open("scripts/web_vocabulary_exceptions.txt", encoding="utf-8"):
    line = line.strip()
    if line and not line.startswith("#"):
        allowed.add(line.split()[0])
budget = int(open("scripts/web_vocabulary_budget.txt", encoding="utf-8").read().strip())
if len(allowed) > budget:
    fail.append(
        f"C. the exception list grew: {len(allowed)} entries against a frozen {budget}.\n"
        "     That file only ever shrinks — adopt the house class instead of adding a name."
    )

app_used: dict[str, set[str]] = {}
for p in sorted(glob.glob("web/app/src/**/*.tsx", recursive=True)):
    for cls in markup_classes(open(p, encoding="utf-8").read()):
        app_used.setdefault(cls, set()).add(p.split("src/")[1])
foreign = {c: f for c, f in app_used.items() if c not in ds_vocabulary and c not in allowed}
if foreign:
    fail.append(
        "C. web/app uses classes the design system does not define and that are not\n"
        "     in the frozen exception list:\n"
        + "\n".join(f"     .{c:22} {', '.join(sorted(f))}" for c, f in sorted(foreign.items()))
    )

if fail:
    print("verify_design_vocabulary: the UI is not speaking one vocabulary.\n", file=sys.stderr)
    for f in fail:
        print("  " + f + "\n", file=sys.stderr)
    sys.exit(1)

print(
    f"verify_design_vocabulary: OK ({len(cards)} cards share one :root, "
    f"{len(ds_vocabulary)} house classes, {len(allowed)} frozen exceptions of {budget})"
)
