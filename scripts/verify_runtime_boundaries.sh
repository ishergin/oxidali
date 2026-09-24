#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() {
  echo "verify_runtime_boundaries: $*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || fail "ripgrep (rg) is required"

check_no_matches() {
  local message="$1"
  shift
  local matches
  matches=$(rg -n "$@" || true)
  if [[ -n "$matches" ]]; then
    fail "$message"$'\n'"$matches"
  fi
}

check_only_allowed_files() {
  local dir="$1"
  local allowed_pattern="$2"
  local files
  files=$(rg --files "$dir" || true)
  if [[ -z "$files" ]]; then
    return 0
  fi
  local unexpected
  unexpected=$(printf '%s\n' "$files" | rg -v "$allowed_pattern" || true)
  if [[ -n "$unexpected" ]]; then
    fail "unexpected files under $dir"$'\n'"$unexpected"
  fi
}

check_no_matches \
  "runtime crates and adapters composition must not use #[path]" \
  '#\[path\s*=' \
  crates/dali2rust-dali-runtime/src \
  crates/dali2rust-display-runtime/src \
  crates/dali2rust-registry-runtime/src \
  crates/dali2rust-operations-runtime/src \
  crates/dali2rust-ws-runtime/src \
  crates/dali2rust-adapters/src/runtime \
  crates/dali2rust-adapters/src/display

check_only_allowed_files \
  crates/dali2rust-adapters/src/runtime \
  '^crates/dali2rust-adapters/src/runtime/(mod|app_router|bus_host|bus_stack|display_source|http_bridges|registry_init|time_settings|workers)\.rs$'

check_only_allowed_files \
  crates/dali2rust-adapters/src/display \
  '^crates/dali2rust-adapters/src/display/mod\.rs$'

if rg --files crates/dali2rust-dali-runtime/src/runtime | rg '/esp_thread\.rs$' >/dev/null 2>&1; then
  fail "shared esp_thread helper must live in dali2rust-bsp, not dali2rust-dali-runtime"
fi

echo "verify_runtime_boundaries: OK"
