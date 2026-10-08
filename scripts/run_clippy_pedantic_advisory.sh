#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOWLIST="${CLIPPY_PEDANTIC_ALLOWLIST:-$ROOT/scripts/clippy_pedantic_allowlist.txt}"
if [[ ! -f "$ALLOWLIST" ]]; then
  echo "error: allowlist not found: $ALLOWLIST" >&2
  exit 1
fi

TARGET="${CLIPPY_HOST_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"

_tmp_allow="$(mktemp)"
trap 'rm -f "$_tmp_allow"' EXIT
grep -vE '^[[:space:]]*($|#)' "$ALLOWLIST" \
  | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
  | grep -v '^$' >"$_tmp_allow" || true

ALLOW_ARGS=()
while IFS= read -r name || [[ -n "$name" ]]; do
  [[ -z "$name" ]] && continue
  if [[ "$name" == clippy::* ]]; then
    ALLOW_ARGS+=("-A" "$name")
  else
    ALLOW_ARGS+=("-A" "clippy::${name}")
  fi
done <"$_tmp_allow"

_allow_count=$(( ${#ALLOW_ARGS[@]} / 2 ))

source "$ROOT/scripts/host_crates.sh"
read_host_crates || exit 1
PACKAGES=("${HOST_CRATES[@]}")

PKG_ARGS=()
for p in "${PACKAGES[@]}"; do
  PKG_ARGS+=("-p" "$p")
done

echo "clippy pedantic (advisory): target=$TARGET allowlist=$ALLOWLIST ($_allow_count allows)"
exec cargo clippy --target "$TARGET" "${PKG_ARGS[@]}" --all-targets -- \
  -W clippy::pedantic \
  -D warnings \
  "${ALLOW_ARGS[@]}"
