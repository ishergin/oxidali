#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "verify_no_bdd_production_hooks: $*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || fail "ripgrep (rg) is required"

matches=$(rg -n 'feature\s*=\s*"bdd"' "$ROOT/crates" "$ROOT/tests/dali2rust-bdd/Cargo.toml" 2>/dev/null || true)
if [[ -n "$matches" ]]; then
  echo "$matches" >&2
  fail "forbidden feature = \"bdd\" in crates or BDD manifest"
fi

matches=$(rg -n 'pub\s+fn\s+bdd_' "$ROOT/crates" 2>/dev/null || true)
if [[ -n "$matches" ]]; then
  echo "$matches" >&2
  fail "forbidden pub fn bdd_* in crates"
fi

matches=$(rg -n 'bdd_support' "$ROOT/crates" 2>/dev/null || true)
if [[ -n "$matches" ]]; then
  echo "$matches" >&2
  fail "forbidden bdd_support reference in crates"
fi

matches=$(rg -n 'registry-test-seed|seed_fixture_' "$ROOT/tests/dali2rust-bdd" 2>/dev/null || true)
if [[ -n "$matches" ]]; then
  echo "$matches" >&2
  fail "registry-test-seed fixture surface referenced from the BDD crate"
fi

echo "verify_no_bdd_production_hooks: OK"
