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
if rg -n "String|Vec<|serde_json::Value" "$ROOT/crates/dali2rust-contracts/src/msg" \
    --glob "!bounded.rs" --glob "!errors.rs" >"$TMP_DIR/contracts.txt"; then
  echo "found forbidden dynamic types in contracts msg:"
  cat "$TMP_DIR/contracts.txt"
  exit 1
fi

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
