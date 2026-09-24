#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BDD_CRATE="$ROOT/tests/dali2rust-bdd"

fail() {
  echo "verify_bdd_layers: $*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || fail "ripgrep (rg) is required"
[[ -d "$BDD_CRATE/src" ]] || fail "missing $BDD_CRATE/src"

FORBIDDEN='_bdd_runtime\b|\.bdd_runtime\(|\bRegistryStore\b|\bRegistryReadPort\b|\bPendingConfirmationSlots\b|\bBusPublisher\b|\bsubscribe_(commands|confirmations|events)\s*\(|registry_seed|registry_read|confirmation_slots|bus_counters\(|display_counters\(|bus_publisher\(|registry_store\(|operation_tracker\(|_runtime\s*\.|BusStackRuntime::'
if matches="$(rg -n --glob '*.rs' --glob '!**/benches/**' "$FORBIDDEN" "$BDD_CRATE")"; then
  echo "$matches" >&2
  fail "forbidden BDD bypass detected under tests/dali2rust-bdd"
fi

if matches="$(rg -n --glob '*.rs' --glob '!**/benches/**' --glob '!**/src/main.rs' 'BusStackRuntime' "$BDD_CRATE")"; then
  echo "$matches" >&2
  fail "BusStackRuntime referenced outside the src/main.rs keep-alive"
fi

echo "verify_bdd_layers: OK"
