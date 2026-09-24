#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOW_FILE="$ROOT/scripts/duplication_clone_budget.txt"
DEFAULT_BUDGET=7

fail_matches() {
  local message="$1"
  local matches="$2"
  if [[ -n "$matches" ]]; then
    echo "verify_duplication: $message" >&2
    echo "$matches" >&2
    exit 1
  fi
}

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_duplication: ripgrep (rg) is required" >&2
  exit 1
fi

operation_begin_flows=$(rg -n 'OperationBeginCommand \{' "$ROOT" -g '*.rs' \
  --glob '!crates/dali2rust-contracts/src/msg/commands.rs' \
  --glob '!crates/dali2rust-api/src/http/handlers/operation_dispatch.rs' \
  --glob '!crates/dali2rust-operations-runtime/tests/**' || true)
fail_matches \
  "operation begin flows must be centralized in crates/dali2rust-api/src/http/handlers/operation_dispatch.rs" \
  "$operation_begin_flows"

budget="$DEFAULT_BUDGET"
if [[ -f "$ALLOW_FILE" ]]; then
  read -r budget <"$ALLOW_FILE" || budget="$DEFAULT_BUDGET"
fi

if ! command -v npx >/dev/null 2>&1; then
  if [[ "${DALI2RUST_SKIP_JSCPD:-0}" == "1" ]]; then
    echo "verify_duplication: SKIPPED by DALI2RUST_SKIP_JSCPD=1 (clone budget unchecked)" >&2
    exit 0
  fi
  echo "verify_duplication: npx not found, so the clone budget was checked against nothing." >&2
  echo "  Install Node, or set DALI2RUST_SKIP_JSCPD=1 to say out loud that this run does not check it." >&2
  exit 1
fi

tmp=$(mktemp)
set +e
npx --yes jscpd@4.0.5 "$ROOT/crates" "$ROOT/tests" \
  --ignore "**/generated/**,**/target/**" \
  --min-lines 25 \
  --min-tokens 120 \
  --reporters "console" >"$tmp" 2>&1
set -e

if ! grep -q 'Found [0-9][0-9]* clones' "$tmp"; then
  echo "verify_duplication: unexpected jscpd output:" >&2
  cat "$tmp" >&2
  exit 1
fi

clones=$(grep -Eo 'Found [0-9]+ clones' "$tmp" | tail -1 | awk '{print $2}')
if [[ -z "${clones:-}" || ! "$clones" =~ ^[0-9]+$ ]]; then
  echo "verify_duplication: could not parse clone count" >&2
  cat "$tmp" >&2
  exit 1
fi

if (( clones > budget )); then
  echo "verify_duplication: clone count $clones exceeds budget $budget (raise budget only with allowlist reason in scripts/duplication_clone_budget.txt)" >&2
  cat "$tmp" >&2
  exit 1
fi

echo "verify_duplication: OK (jscpd clones=$clones budget=$budget)"
