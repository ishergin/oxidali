#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_fixed_bus_guardrails: ripgrep (rg) is required" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[guardrail] checking contracts msg for dynamic payload types..."
python3 - "$ROOT/crates/dali2rust-contracts/src/msg" <<'PY'
import re
import sys
from pathlib import Path

msg = Path(sys.argv[1])
allowed_files = {"bounded.rs", "errors.rs"}
test_module = re.compile(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*(;|\{)")
literal_or_comment = re.compile(
    r'//[^\n]*|/\*.*?\*/|r(#*)".*?"\1|b?"(?:\\.|[^"\\])*"|b?\'(?:\\.|[^\'\\])\'',
    re.DOTALL,
)
forbidden = re.compile(r"String|Vec<|serde_json::Value")


def blank(text):
    return re.sub(r"[^\n]", " ", text)


def test_only_files():
    names = set()
    for source in msg.glob("*.rs"):
        for name, end in test_module.findall(source.read_text(encoding="utf-8")):
            if end == ";":
                names.add(f"{name}.rs")
    return names


def production_text(text):
    masked = literal_or_comment.sub(lambda m: blank(m.group(0)), text)
    spans = []
    for found in test_module.finditer(masked):
        if found.group(2) != "{":
            continue
        depth, cursor = 1, found.end()
        while depth and cursor < len(masked):
            depth += {"{": 1, "}": -1}.get(masked[cursor], 0)
            cursor += 1
        spans.append((found.start(), cursor))
    for start, end in reversed(spans):
        text = text[:start] + blank(text[start:end]) + text[end:]
    return text


skipped = allowed_files | test_only_files()
findings = []
for source in sorted(msg.glob("*.rs")):
    if source.name in skipped:
        continue
    for number, line in enumerate(production_text(source.read_text(encoding="utf-8")).splitlines(), 1):
        if forbidden.search(line):
            findings.append(f"{source}:{number}: {line.strip()}")
if findings:
    print("found forbidden dynamic types in contracts msg:")
    print("\n".join(findings))
    sys.exit(1)
PY

echo "[guardrail] checking runtime registry for json value leakage..."
if rg -n "serde_json|json!|\\bValue\\b|obs_value_source: Option<String>|obs_last_dapc_source: Option<String>" \
    "$ROOT/crates/dali2rust-registry-runtime/src/runtime/registry" \
    --glob "!persistence.rs" >"$TMP_DIR/registry.txt"; then
  echo "found forbidden runtime registry patterns:"
  cat "$TMP_DIR/registry.txt"
  exit 1
fi

echo "[guardrail] checking non-api crates for serde_json leakage..."
if rg -n "serde_json" \
    "$ROOT/crates/dali2rust-adapters/src/runtime" \
    "$ROOT/crates/dali2rust-bsp/src" \
    "$ROOT/crates/dali2rust-bus/src" \
    "$ROOT/crates/dali2rust-contracts/src" \
    "$ROOT/crates/dali2rust-dali-runtime/src" \
    "$ROOT/crates/dali2rust-display-runtime/src" \
    "$ROOT/crates/dali2rust-operations-runtime/src" \
    "$ROOT/crates/dali2rust-registry-runtime/src" \
    "$ROOT/crates/dali2rust-ws-runtime/src" \
    "$ROOT/crates/dali2rust-mqtt-runtime/src" \
    --glob "!**/tests/**" --glob "!**/*test*.rs" --glob "!**/registry/persistence.rs" \
    >"$TMP_DIR/serde.txt"; then
  echo "found forbidden serde_json usage outside API boundary:"
  cat "$TMP_DIR/serde.txt"
  exit 1
fi

echo "[guardrail] fixed-size constraints look good"
