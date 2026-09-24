#!/usr/bin/env bash
cd "$(dirname "$0")/.."
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET_FILE="$ROOT/scripts/fn_length_budget.txt"
DEFAULT_BUDGET=0

TARGET="${DALI2RUST_HOST_TARGET:-$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')}"
TARGET="${TARGET:-aarch64-apple-darwin}"

HOST_CRATES_FILE="$ROOT/scripts/host_crates.txt"
if [[ ! -f "$HOST_CRATES_FILE" ]]; then
  echo "verify_fn_length: missing $HOST_CRATES_FILE" >&2
  exit 1
fi
HOST_CRATES=()
while IFS= read -r line; do
  line="${line%%#*}"
  line="${line//[[:space:]]/}"
  [[ -n "$line" ]] && HOST_CRATES+=("$line")
done <"$HOST_CRATES_FILE"
if (( ${#HOST_CRATES[@]} == 0 )); then
  echo "verify_fn_length: $HOST_CRATES_FILE lists no crates" >&2
  exit 1
fi

budget="$DEFAULT_BUDGET"
if [[ -f "$BUDGET_FILE" ]]; then
  read -r budget <"$BUDGET_FILE" || budget="$DEFAULT_BUDGET"
fi

pkg_args=()
for c in "${HOST_CRATES[@]}"; do
  pkg_args+=(-p "$c")
done

tmp=$(mktemp)
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
  echo "New >40-line functions are forbidden — decompose them, or (only when a" >&2
  echo "refactor legitimately removes one) lower scripts/fn_length_budget.txt." >&2
  grep -A2 'this function has too many lines' "$tmp" >&2 || true
  exit 1
fi

if (( count < budget )); then
  echo "verify_fn_length: OK ($count < budget $budget) — lower the baseline in" >&2
  echo "scripts/fn_length_budget.txt to $count to lock in the improvement." >&2
fi

echo "verify_fn_length: OK (too_many_lines=$count budget=$budget)"
