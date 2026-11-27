#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DS_DIR="$ROOT/web/design-system"
STAMP_NAME=".pushed.sha256"
STAMP_FILE="$DS_DIR/$STAMP_NAME"
DESIGN_SYSTEM_URL="https://claude.ai/artifact/Pp9mKJk2akZYummf1CbfZP"

die() {
  echo "verify_design_system_pushed: $*" >&2
  exit 1
}

current_manifest() {
  (
    cd "$DS_DIR"
    find . -type f \
      ! -name "$STAMP_NAME" \
      ! -name '_ds_*' \
      ! -name '_adherence.*' \
      -print0
  ) | LC_ALL=C sort -z | while IFS= read -r -d '' rel; do
    rel="${rel#./}"
    printf '%s  %s\n' "$(shasum -a 256 "$DS_DIR/$rel" | cut -d' ' -f1)" "$rel"
  done
}

[[ -d "$DS_DIR" ]] || die "missing design system dir: $DS_DIR"

if [[ "${1:-}" == "--stamp" ]]; then
  current_manifest >"$STAMP_FILE"
  echo "verify_design_system_pushed: stamped $(wc -l <"$STAMP_FILE" | tr -d ' ') files as pushed to $DESIGN_SYSTEM_URL"
  echo "  commit $STAMP_NAME together with the cards it covers."
  exit 0
fi

if [[ $# -gt 0 ]]; then
  die "unknown argument '$1' (expected no arguments, or --stamp)"
fi

if [[ ! -f "$STAMP_FILE" ]]; then
  die "missing push stamp $STAMP_FILE — push the design system, then re-run with --stamp"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

current_manifest | LC_ALL=C sort >"$tmp/current"
LC_ALL=C sort "$STAMP_FILE" >"$tmp/stamp"

cut -d' ' -f3- "$tmp/current" | LC_ALL=C sort >"$tmp/paths_current"
cut -d' ' -f3- "$tmp/stamp" | LC_ALL=C sort >"$tmp/paths_stamp"

comm -13 "$tmp/paths_stamp" "$tmp/paths_current" >"$tmp/added"
comm -23 "$tmp/paths_stamp" "$tmp/paths_current" >"$tmp/deleted"

comm -13 "$tmp/stamp" "$tmp/current" | cut -d' ' -f3- | LC_ALL=C sort >"$tmp/moved"
comm -12 "$tmp/paths_stamp" "$tmp/paths_current" >"$tmp/known"
comm -12 "$tmp/known" "$tmp/moved" >"$tmp/edited"

total_drift=$(cat "$tmp/added" "$tmp/deleted" "$tmp/edited" | wc -l | tr -d ' ')
if [[ "$total_drift" == "0" ]]; then
  echo "verify_design_system_pushed: OK ($(wc -l <"$tmp/current" | tr -d ' ') cards match the last push)."
  exit 0
fi

echo "verify_design_system_pushed: web/design-system/ has changes that were never pushed." >&2
while IFS= read -r p; do [[ -n "$p" ]] && echo "  new, never pushed:   $p" >&2; done <"$tmp/added"
while IFS= read -r p; do [[ -n "$p" ]] && echo "  edited since push:   $p" >&2; done <"$tmp/edited"
while IFS= read -r p; do [[ -n "$p" ]] && echo "  deleted, still live: $p" >&2; done <"$tmp/deleted"
echo >&2
echo "  The design system is canonical and its artifact is where it lives, so a card" >&2
echo "  that exists only here is a UI change nobody can review. Publish it with the" >&2
echo "  Artifact tool to $DESIGN_SYSTEM_URL:" >&2
echo "  each card of the first two lists at project/<its path here> as text/plain," >&2
echo "  each of the third mapped to null. Read the cards back, then re-run this" >&2
echo "  script with --stamp and commit $STAMP_NAME alongside" >&2
echo "  (documentation/architecture/10-build-release-and-tooling.md, Embedded web UI)." >&2
exit 1
