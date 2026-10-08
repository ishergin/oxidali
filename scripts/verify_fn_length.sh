#!/usr/bin/env bash
cd "$(dirname "$0")/.."
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET_FILE="$ROOT/scripts/fn_length_budget.txt"

TARGET="${DALI2RUST_HOST_TARGET:-$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')}"
TARGET="${TARGET:-aarch64-apple-darwin}"

source "$ROOT/scripts/host_crates.sh"
read_host_crates || exit 1

budget=""
if [[ -f "$BUDGET_FILE" ]]; then
  IFS= read -r budget <"$BUDGET_FILE" || true
fi
budget="${budget//[[:space:]]/}"
if [[ ! "$budget" =~ ^[0-9]+$ ]]; then
  echo "verify_fn_length: $BUDGET_FILE must hold one non-negative integer, found '${budget}'" >&2
  exit 1
fi

pkg_args=()
for c in "${HOST_CRATES[@]}"; do
  pkg_args+=(-p "$c")
done

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
set +e
cargo clippy --target "$TARGET" "${pkg_args[@]}" --lib \
  -- -W clippy::too_many_lines >"$tmp" 2>&1
status=$?
set -e

if (( status != 0 )); then
  echo "verify_fn_length: clippy failed to run (compile error?)" >&2
  cat "$tmp" >&2
  exit 1
fi

count=$(grep -c 'this function has too many lines' "$tmp" || true)
count="${count:-0}"

if (( count > budget )); then
  echo "verify_fn_length: $count functions exceed 40 lines, budget is $budget" >&2
  echo "A function over 40 lines is forbidden — decompose it." >&2
  grep -A2 'this function has too many lines' "$tmp" >&2 || true
  exit 1
fi

if (( count < budget )); then
  echo "verify_fn_length: only $count functions exceed 40 lines, budget is $budget —" >&2
  echo "lower scripts/fn_length_budget.txt to $count; the baseline only goes down." >&2
  exit 1
fi

echo "verify_fn_length: OK (too_many_lines=$count budget=$budget)"
