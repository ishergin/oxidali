#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() {
  echo "verify_native_contracts: $*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || die "ripgrep (rg) is required"

if [[ ! -d "$ROOT/crates/dali2rust-contracts/src/msg" ]]; then
  die "missing native contracts dir: $ROOT/crates/dali2rust-contracts/src/msg"
fi

if rg -n 'flatbuffers' "$ROOT/crates" -g 'Cargo.toml' --glob '!**/target/**' 2>/dev/null | grep -q .; then
  die "flatbuffers must not appear in workspace crate Cargo.toml files"
fi

if rg -q '^name = "flatbuffers"' "$ROOT/Cargo.lock" 2>/dev/null; then
  die "flatbuffers must not be present in Cargo.lock"
fi

if compgen -G "$ROOT/schemas/**/*.fbs" >/dev/null 2>&1; then
  die "legacy FlatBuffers schemas must be removed from $ROOT/schemas/"
fi

forbidden_generated=$(rg -n 'dali2rust_contracts::generated|::generated::|mod generated' \
  "$ROOT/crates" "$ROOT/tests" -g '*.rs' || true)
if [[ -n "$forbidden_generated" ]]; then
  die "forbidden generated/FlatBuffers-era module paths:\n$forbidden_generated"
fi

if rg -q 'compat_bytes' "$ROOT/crates" "$ROOT/tests" -g '*.rs'; then
  die "compat_bytes surface removed — references must be deleted"
fi

legacy_bytes=$(rg -n 'build_[A-Za-z0-9_]+_bytes\(' "$ROOT/crates" "$ROOT/tests" -g '*.rs' || true)
if [[ -n "$legacy_bytes" ]]; then
  die "use typed builders (no *_bytes) in crates/tests:\n$legacy_bytes"
fi

from_slice_prod=$(
  rg -n 'BusFrame::from_slice' \
    "$ROOT/crates/dali2rust-api" \
    "$ROOT/crates/dali2rust-adapters/src/runtime" \
    "$ROOT/tests/dali2rust-bdd/src" \
    -g '*.rs' || true
)
if [[ -n "$from_slice_prod" ]]; then
  die "BusFrame::from_slice forbidden outside bus codec tests — use typed BusFrame::command/event/confirmation:\n$from_slice_prod"
fi

echo "verify_native_contracts: OK (native contracts policy)."
