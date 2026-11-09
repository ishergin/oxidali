#!/usr/bin/env python3
import glob
import re
import sys

ANY_DATE = re.compile(r"\b20\d\d[-/.]\d{1,2}[-/.]\d{1,2}\b|\b\d{1,2}\.\d{1,2}\.20\d\d\b")
TIME_AFTER = re.compile(r"[ T]\d{1,2}:\d\d")
COMMENT = re.compile(r"<!--.*?-->|/\*.*?\*/", re.S)


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


def history_dates(text: str) -> list[tuple[int, str]]:
    comments = [m.span() for m in COMMENT.finditer(text)]
    out = []
    for m in ANY_DATE.finditer(text):
        in_comment = any(a <= m.start() < b for a, b in comments)
        if in_comment or not TIME_AFTER.match(text, m.end()):
            out.append((text.count("\n", 0, m.start()) + 1, m.group(0)))
    return out


PROPERTY_SET = re.compile(r"(--[A-Za-z][\w-]*)\s*:")
BARE_VAR = re.compile(r"var\(\s*(--[A-Za-z][\w-]*)\s*\)")


def undeclared_vars(path: str, declared: set[str]) -> list[str]:
    text = open(path, encoding="utf-8").read()
    known = declared | set(PROPERTY_SET.findall(text))
    out = []
    for m in BARE_VAR.finditer(text):
        if m.group(1) not in known:
            line = text.count("\n", 0, m.start()) + 1
            out.append(f"     {path}:{line}: {m.group(1)}")
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

dated = []
for p in cards + ["web/design-system/tokens.css"]:
    for number, date in history_dates(open(p, encoding="utf-8").read()):
        dated.append(f"     {p}:{number}: {date}")
if dated:
    fail.append(
        "D. dated text in a card (a card shows the design as it is; its history goes in the\n"
        "     commit; only the mocked UI shows a date, always with its time, never a comment):\n"
        + "\n".join(dated)
    )

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

app_sources = sorted(
    p for ext in ("tsx", "ts", "css") for p in glob.glob(f"web/app/src/**/*.{ext}", recursive=True)
)
app_declared: set[str] = set()
for p in app_sources:
    app_declared |= set(PROPERTY_SET.findall(open(p, encoding="utf-8").read()))
unbound = [line for p in app_sources for line in undeclared_vars(p, app_declared)]
unbound += [line for p in cards for line in undeclared_vars(p, set())]
if unbound:
    fail.append(
        "E. var() with no fallback names a custom property nothing declares, so the\n"
        "     property silently falls back to its initial value:\n" + "\n".join(unbound)
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
