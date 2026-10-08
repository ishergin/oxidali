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
    >"$TMP_DIR/registry.txt"; then
  echo "found forbidden runtime registry patterns:"
  cat "$TMP_DIR/registry.txt"
  exit 1
fi

echo "[guardrail] checking that only dali2rust-api depends on serde_json..."
python3 - "$ROOT" <<'PY'
import sys
import tomllib
from pathlib import Path

root = Path(sys.argv[1])
sys.path.insert(0, str(root / "scripts"))
from host_crates import host_crates

JSON_CRATE = "dali2rust-api"
FORBIDDEN = "serde_json"


def runtime_tables(manifest):
    yield "dependencies", manifest.get("dependencies", {})
    for target, spec in manifest.get("target", {}).items():
        yield f"target.{target}.dependencies", spec.get("dependencies", {})


def names_serde_json(key, spec):
    package = spec.get("package", key) if isinstance(spec, dict) else key
    return FORBIDDEN in (key, package)


findings = []
for crate in host_crates() + ["dali2rust-firmware"]:
    if crate == JSON_CRATE:
        continue
    path = root / "crates" / crate / "Cargo.toml"
    manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    for table, deps in runtime_tables(manifest):
        findings.extend(
            f"{path.relative_to(root)}: [{table}] {key}"
            for key, spec in deps.items()
            if names_serde_json(key, spec)
        )
if findings:
    print("JSON belongs to dali2rust-api; these crates depend on serde_json outside dev-dependencies:")
    print("\n".join(findings))
    sys.exit(1)
PY

echo "[guardrail] fixed-size constraints look good"
