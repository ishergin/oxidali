#!/usr/bin/env python3
import argparse
import ast
import io
import json
import re
import subprocess
import sys
import tokenize
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUDGET_FILE = ROOT / "scripts" / "comment_budget.txt"
TS_HELPER = ROOT / "scripts" / "ts_comments.mjs"
MAX_MARKER_LEN = 120

EXCLUDED_PREFIXES = (
    "web/design-system/",
    "crates/dali2rust-firmware/assets/",
    "tools/hil/corpus/",
)

C_LIKE_MARKERS = [
    re.compile(r"^// SAFETY: \S.*$"),
    re.compile(r"^// sleep-ok: \S.*$"),
    re.compile(r"^// busy-wait-ok: \S.*$"),
    re.compile(
        r"^// (?:IEC 62386-\d{3}(?: AMD\d)?|DiiA \S+) "
        r"(?:§[\d.]+[a-z]?|Table \d+|Annex [A-Z])"
        r"(?:, (?:§[\d.]+[a-z]?|Table \d+|Annex [A-Z]))*$"
    ),
]
BDD_ID = r"[A-Z][A-Z0-9]*(?:-[A-Z][A-Z0-9]*)*-\d+[a-z]?"
BDD_ID_LINE = re.compile(rf"^// {BDD_ID}(?:,? {BDD_ID})*$")
BDD_STEPS = "tests/dali2rust-bdd/src/steps/"
SAFETY_DOC_HEAD = "/// # Safety"
TS_MARKERS = [re.compile(r"^// @ts-expect-error\b.*$")]
HASH_MARKERS = [
    re.compile(r"^#!"),
    re.compile(r"^# shellcheck \S.*$"),
    re.compile(r"^# language: \S+$"),
    re.compile(r"^# CONFIG_[A-Z0-9_]+ is not set$"),
]


@dataclass
class Comment:
    start: int
    end: int
    text: str
    block: bool


def kind_of(rel):
    name = rel.rsplit("/", 1)[-1]
    if rel.endswith(".rs"):
        return "rust"
    if rel.endswith(".c"):
        return "c"
    if rel.endswith((".ts", ".tsx")):
        return "ts"
    if rel.endswith(".css"):
        return "css"
    if rel.endswith(".py"):
        return "python"
    if rel.endswith(".sh"):
        return "shell"
    if name == "justfile":
        return "just"
    if rel.endswith(".toml"):
        return "toml"
    if rel.endswith((".yml", ".yaml")):
        return "yaml"
    if rel.endswith(".feature"):
        return "gherkin"
    if rel.endswith(".defaults"):
        return "sdkconfig"
    if rel.endswith(".csv"):
        return "csv"
    if rel.endswith(".example"):
        return "env"
    return None


def is_ident(ch):
    return ch.isalnum() or ch == "_"


def skip_quoted(src, i, quote):
    n = len(src)
    j = i + 1
    while j < n and src[j] != quote:
        j += 2 if src[j] == "\\" else 1
    return j + 1


def raw_string_end(src, i):
    n = len(src)
    k = i + 1 if src[i] in "bc" and i + 1 < n and src[i + 1] == "r" else i
    if src[k] != "r":
        return None
    h = k + 1
    while h < n and src[h] == "#":
        h += 1
    if h >= n or src[h] != '"':
        return None
    close = '"' + "#" * (h - k - 1)
    j = src.find(close, h + 1)
    return n if j == -1 else j + len(close)


def block_comment_end(src, i, nested):
    n = len(src)
    depth = 1
    j = i + 2
    while j < n and depth:
        if nested and src.startswith("/*", j):
            depth += 1
            j += 2
        elif src.startswith("*/", j):
            depth -= 1
            j += 2
        else:
            j += 1
    return j


def char_literal_end(src, i, rust):
    n = len(src)
    if i + 1 < n and src[i + 1] == "\\":
        j = i + 2
        while j < n and src[j] != "'":
            j += 2 if src[j] == "\\" else 1
        return j + 1
    if i + 2 < n and src[i + 2] == "'":
        return i + 3
    if rust:
        return None
    return skip_quoted(src, i, "'")


def lex_c_like(src, rust):
    comments = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j == -1 else j
            comments.append(Comment(i, j, src[i:j], False))
            i = j
            continue
        if src.startswith("/*", i):
            j = block_comment_end(src, i, rust)
            comments.append(Comment(i, j, src[i:j], True))
            i = j
            continue
        if rust and c in "rbc" and not (i > 0 and is_ident(src[i - 1])):
            end = raw_string_end(src, i)
            if end is not None:
                i = end
                continue
            if c in "bc" and i + 1 < n and src[i + 1] in "\"'":
                i += 1
                c = src[i]
        if c == '"':
            i = skip_quoted(src, i, '"')
            continue
        if c == "'":
            end = char_literal_end(src, i, rust)
            i = end if end is not None else i + 1
            continue
        i += 1
    return comments


def lex_css(src):
    comments = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j == -1 else j + 2
            comments.append(Comment(i, j, src[i:j], True))
            i = j
            continue
        if c in "\"'":
            i = skip_quoted(src, i, c)
            continue
        i += 1
    return comments


def lex_ts(src, tsx=True):
    res = subprocess.run(
        ["node", str(TS_HELPER), "tsx" if tsx else "ts"],
        input=src, capture_output=True, text=True, cwd=ROOT / "web" / "app",
    )
    if res.returncode != 0:
        raise RuntimeError(res.stderr.strip())
    return [Comment(a, b, src[a:b], src.startswith("/*", a)) for a, b in json.loads(res.stdout)]


def toml_comment_pos(line, quote):
    i = 0
    n = len(line)
    while i < n:
        ch = line[i]
        if quote:
            if len(quote) == 3 and line.startswith(quote, i):
                i += 3
                quote = None
                continue
            if ch == "\\" and quote in ('"', '"""'):
                i += 2
                continue
            if len(quote) == 1 and ch == quote:
                quote = None
            i += 1
            continue
        if line.startswith('"""', i) or line.startswith("'''", i):
            quote = line[i:i + 3]
            i += 3
            continue
        if ch in "\"'":
            quote = ch
        elif ch == "#":
            return i, None
        i += 1
    return None, quote if quote and len(quote) == 3 else None


def shell_comment_pos(body):
    quote = None
    i = 0
    while i < len(body):
        ch = body[i]
        if quote:
            if ch == "\\" and quote == '"':
                i += 2
                continue
            if ch == quote:
                quote = None
        elif ch in "\"'":
            quote = ch
        elif ch == "\\":
            i += 2
            continue
        elif ch == "#" and (i == 0 or body[i - 1] in " \t;(|&"):
            return i
        i += 1
    return None


def yaml_comment_pos(body):
    quote = None
    for i, ch in enumerate(body):
        if quote:
            if ch == quote:
                quote = None
        elif ch in "\"'" and (i == 0 or body[i - 1] in " :[{,-"):
            quote = ch
        elif ch == "#" and (i == 0 or body[i - 1] in " \t"):
            return i
    return None


def lex_hash(src, kind):
    comments = []
    offset = 0
    toml_quote = None
    heredoc = None
    in_docstring = False
    for line in src.splitlines(keepends=True):
        body = line.rstrip("\n")
        stripped = body.strip()
        pos = None
        if heredoc:
            if stripped == heredoc:
                heredoc = None
        elif kind == "gherkin":
            if stripped.startswith('"""') or stripped.startswith("```"):
                in_docstring = not in_docstring
            elif not in_docstring and stripped.startswith("#"):
                pos = body.index("#")
        elif kind in ("sdkconfig", "csv", "env"):
            if stripped.startswith("#"):
                pos = body.index("#")
        elif kind in ("shell", "just"):
            pos = shell_comment_pos(body)
            m = re.search(r"<<-?\s*['\"]?([A-Za-z_]+)['\"]?", body if pos is None else body[:pos])
            if m:
                heredoc = m.group(1)
        elif kind == "yaml":
            pos = yaml_comment_pos(body)
        else:
            pos, toml_quote = toml_comment_pos(body, toml_quote)
        if pos is not None:
            comments.append(Comment(offset + pos, offset + len(body), body[pos:], False))
        offset += len(line)
    return comments


def line_bounds(src, pos):
    ls = src.rfind("\n", 0, pos) + 1
    le = src.find("\n", pos)
    return ls, len(src) if le == -1 else le


def group_units(src, comments):
    units = []
    idx = 0
    while idx < len(comments):
        c = comments[idx]
        ls, le = line_bounds(src, c.start)
        alone = src[ls:c.start].strip() == "" and src[c.end:le].strip() == ""
        if c.block or not alone:
            units.append(([c], alone))
            idx += 1
            continue
        group = [c]
        j = idx + 1
        while j < len(comments):
            d = comments[j]
            dls, dle = line_bounds(src, d.start)
            if d.block or dls != line_bounds(src, group[-1].start)[1] + 1:
                break
            if src[dls:d.start].strip() or src[d.end:dle].strip():
                break
            group.append(d)
            j += 1
        units.append((group, True))
        idx = j
    return units


def markers_for(kind):
    if kind in ("rust", "c"):
        return C_LIKE_MARKERS
    if kind == "ts":
        return TS_MARKERS
    return HASH_MARKERS


def marker_match(markers, text):
    return len(text) <= MAX_MARKER_LEN and any(m.match(text) for m in markers)


def keep_safety_doc(texts, keep, k):
    keep[k] = True
    j = k + 1
    while j < len(texts) and texts[j].startswith("///") and texts[j] != "///":
        keep[j] = True
        j += 1
    return j, max(0, j - k - 2)


def keep_marker(kind, texts, keep, k, markers):
    keep[k] = True
    j = k + 1
    t = texts[k]
    continues = kind in ("rust", "c") and not t.startswith(("// IEC", "// DiiA"))
    while continues and j < len(texts) and texts[j].startswith("//") \
            and not texts[j].startswith("///") and not marker_match(markers, texts[j]):
        keep[j] = True
        j += 1
    return j, j - k - 1


def classify_group(rel, kind, group):
    texts = [g.text.strip() for g in group]
    keep = [False] * len(texts)
    extra = 0
    markers = markers_for(kind)
    k = 0
    while k < len(texts):
        t = texts[k]
        if kind == "rust" and t == SAFETY_DOC_HEAD:
            k, more = keep_safety_doc(texts, keep, k)
            extra += more
            continue
        if marker_match(markers, t):
            k, more = keep_marker(kind, texts, keep, k, markers)
            extra += more
            continue
        if kind == "rust" and rel.startswith(BDD_STEPS) and BDD_ID_LINE.match(t):
            keep[k] = True
        k += 1
    return keep, extra


def evaluate(rel, kind, src, lexer):
    comments = lexer(src)
    inline = []
    line_removals = []
    violations = 0
    multi = 0
    markers = markers_for(kind)
    for group, alone in group_units(src, comments):
        if not alone:
            c = group[0]
            if not c.block and marker_match(markers, c.text.strip()):
                continue
            violations += 1
            inline.append(c)
            continue
        keep, extra = classify_group(rel, kind, group)
        if extra:
            violations += extra
            multi += 1
        for c, k in zip(group, keep):
            if not k:
                violations += src[c.start:c.end].count("\n") + 1
                line_removals.append(("line", c))
    if inline:
        rest, more, out = evaluate(rel, kind, remove_inline(src, inline), lexer)
        return len(inline) + rest, more, out
    return violations, multi, apply_removals(src, line_removals)


def remove_inline(src, comments):
    for c in sorted(comments, key=lambda c: -c.start):
        ls = src.rfind("\n", 0, c.start) + 1
        a = c.start
        while a > ls and src[a - 1] in " \t":
            a -= 1
        b = c.end
        before = src[a - 1] if a > 0 else ""
        after = src[b] if b < len(src) else ""
        sep = " " if before and after and not before.isspace() and not after.isspace() else ""
        if a == ls:
            while b < len(src) and src[b] in " \t":
                b += 1
        src = src[:a] + sep + src[b:]
    return src


def apply_removals(src, removals):
    if not removals:
        return src
    lines = src.splitlines(keepends=True)
    drop = set()
    for _, c in removals:
        drop.update(range(src.count("\n", 0, c.start), src.count("\n", 0, c.end) + 1))
    out = {idx: text for idx, text in enumerate(lines) if idx not in drop}
    return tidy(len(lines), out, drop)


def is_blank(t):
    return t.strip() == ""


def dropped_runs(n, drop):
    runs = []
    idx = 0
    while idx < n:
        if idx in drop:
            j = idx
            while j + 1 < n and j + 1 in drop:
                j += 1
            runs.append((idx, j))
            idx = j + 1
        else:
            idx += 1
    return runs


def tidy(n, out, drop):
    for a, b in dropped_runs(n, drop):
        prev = next((k for k in range(a - 1, -1, -1) if k in out), None)
        nxt = next((k for k in range(b + 1, n) if k in out), None)
        if nxt is not None and is_blank(out[nxt]):
            opener = prev is not None and out[prev].rstrip().endswith(("{", "(", "[", ":"))
            if prev is None or is_blank(out[prev]) or opener:
                del out[nxt]
                continue
        if prev is not None and is_blank(out[prev]):
            if nxt is None or out[nxt].lstrip().startswith(("}", ")", "]")):
                del out[prev]
    return "".join(out[k] for k in sorted(out))


def without_comments(src, comments):
    parts = []
    pos = 0
    for c in comments:
        parts.append(src[pos:c.start])
        parts.append(" ")
        pos = c.end
    parts.append(src[pos:])
    return re.sub(r"\s+", " ", "".join(parts)).strip()


def python_docstrings(tree):
    found = []
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            body = node.body
            if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) \
                    and isinstance(body[0].value.value, str):
                found.append((node, body[0]))
    return found


def python_signature(src):
    tree = ast.parse(src)
    for owner, _ in python_docstrings(tree):
        owner.body = owner.body[1:] or [ast.Pass()]
    return ast.dump(tree, include_attributes=False)


def python_comments(src):
    comments = []
    starts = [0]
    for ln in src.splitlines(keepends=True):
        starts.append(starts[-1] + len(ln))
    for tok in tokenize.generate_tokens(io.StringIO(src).readline):
        if tok.type != tokenize.COMMENT:
            continue
        (row, col), (_, ecol) = tok.start, tok.end
        if row == 1 and tok.string.startswith("#!"):
            continue
        comments.append(Comment(starts[row - 1] + col, starts[row - 1] + ecol, tok.string, False))
    return comments


def strip_python_docstrings(src):
    docs = python_docstrings(ast.parse(src))
    if not docs:
        return 0, src
    lines = src.splitlines(keepends=True)
    drop = set()
    replace = {}
    count = 0
    for owner, expr in docs:
        first, last = expr.lineno - 1, expr.end_lineno - 1
        count += last - first + 1
        if len(owner.body) == 1 and not isinstance(owner, ast.Module):
            replace[first] = re.match(r"\s*", lines[first]).group(0) + "pass\n"
            drop.update(range(first + 1, last + 1))
        else:
            drop.update(range(first, last + 1))
    out = {i: t for i, t in enumerate(lines) if i not in drop}
    out.update(replace)
    return count, tidy(len(lines), out, drop)


def evaluate_python(rel, src):
    violations, multi, out = evaluate(rel, "python", src, python_comments)
    if "__doc__" in src:
        return violations + sum(e.end_lineno - e.lineno + 1 for _, e in python_docstrings(ast.parse(out))), multi + 1, out
    count, out = strip_python_docstrings(out)
    return violations + count, multi, out


def evaluate_file(rel, src):
    kind = kind_of(rel)
    if kind == "python":
        return evaluate_python(rel, src)
    return evaluate(rel, kind, src, lambda text: comments_of(kind, text, rel))


def comments_of(kind, src, rel=""):
    if kind in ("rust", "c"):
        return lex_c_like(src, kind == "rust")
    if kind == "css":
        return lex_css(src)
    if kind == "ts":
        return lex_ts(src, rel.endswith(".tsx"))
    return lex_hash(src, kind)


def signature(rel, src):
    kind = kind_of(rel)
    if kind == "python":
        return python_signature(src)
    return without_comments(src, comments_of(kind, src, rel))


def git_files():
    res = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True)
    if res.returncode != 0:
        return [str(f.relative_to(ROOT)) for f in sorted(ROOT.rglob("*"))
                if f.is_file() and "target" not in f.parts and "node_modules" not in f.parts]
    return [line for line in res.stdout.splitlines() if line]


def selected(paths):
    tracked = git_files()
    if paths:
        prefixes = [str((Path.cwd() / p).resolve().relative_to(ROOT)).rstrip("/") for p in paths]
        tracked = [f for f in tracked if any(f == p or f.startswith(p + "/") for p in prefixes)]
    return [f for f in tracked if kind_of(f) and not f.startswith(EXCLUDED_PREFIXES) and (ROOT / f).exists()]


def read_budget():
    if not BUDGET_FILE.exists():
        return 0
    return int(BUDGET_FILE.read_text().split()[0])


def run(paths, strip):
    total = 0
    per_file = []
    mismatched = []
    for rel in selected(paths):
        path = ROOT / rel
        src = path.read_text(encoding="utf-8")
        violations, multi, out = evaluate_file(rel, src)
        if strip and out != src:
            if signature(rel, out) != signature(rel, src):
                mismatched.append(rel)
            else:
                path.write_text(out, encoding="utf-8")
                violations, multi, _ = evaluate_file(rel, out)
        total += violations
        if violations:
            per_file.append((violations, rel, multi))
    per_file.sort(reverse=True)
    return total, per_file, mismatched


def report_budget(total, per_file):
    budget = read_budget()
    if total > budget:
        for v, rel, _ in per_file[:20]:
            print(f"  {v:6d} {rel}", file=sys.stderr)
        print(f"verify_comments: {total} comment lines outside the allowed one-line markers, "
              f"budget {budget}", file=sys.stderr)
        return 1
    if total < budget:
        print(f"verify_comments: OK ({total} < budget {budget}; lower scripts/comment_budget.txt to {total})")
    else:
        print(f"verify_comments: OK ({total} comment lines outside markers, budget {budget})")
    return 0


def main():
    global ROOT
    ap = argparse.ArgumentParser(description="Comments are banned except one-line markers.")
    ap.add_argument("paths", nargs="*")
    ap.add_argument("--strip", action="store_true", help="remove every comment that is not a marker")
    ap.add_argument("--list", action="store_true", help="per-file counts")
    ap.add_argument("--root", help="alternative repository root")
    args = ap.parse_args()
    if args.root:
        ROOT = Path(args.root).resolve()
    total, per_file, mismatched = run(args.paths, args.strip)
    if args.list:
        for v, rel, multi in per_file:
            print(f"{v:6d} {rel}" + (f"  ({multi} multi-line markers)" if multi else ""))
    for rel in mismatched:
        print(f"verify_comments: {rel}: code would change, file left untouched", file=sys.stderr)
    if args.paths or args.strip:
        print(f"verify_comments: {total} comment lines outside the allowed one-line markers")
        return 1 if mismatched else 0
    return report_budget(total, per_file)


if __name__ == "__main__":
    sys.exit(main())
