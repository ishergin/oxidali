#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

APP_DIR="$ROOT/web/app"
FW_ASSETS_DIR="$ROOT/crates/dali2rust-firmware/assets/web"
STAMP_FILE="$FW_ASSETS_DIR/.sources.sha256"

die() {
  echo "verify_web_mirror_fresh: $*" >&2
  exit 1
}

source_fingerprint() {
  (
    cd "$APP_DIR"
    find src public -type f -print0 2>/dev/null
    printf '%s\0' index.html package.json package-lock.json vite.config.ts
    printf '%s\0' tsconfig.json tsconfig.app.json tsconfig.node.json
  ) | LC_ALL=C sort -z | while IFS= read -r -d '' rel; do
    [[ -f "$APP_DIR/$rel" ]] || continue
    printf '%s ' "$rel"
    shasum -a 256 "$APP_DIR/$rel" | cut -d' ' -f1
  done | shasum -a 256 | cut -d' ' -f1
}

if [[ ! -f "$STAMP_FILE" ]]; then
  die "missing source stamp $STAMP_FILE — run scripts/build_web_ui.sh and land the mirror"
fi

expected="$(source_fingerprint)"
recorded="$(tr -d '[:space:]' <"$STAMP_FILE")"

if [[ "$expected" != "$recorded" ]]; then
  echo "verify_web_mirror_fresh: the embedded UI bundle is older than web/app." >&2
  echo "  recorded: $recorded" >&2
  echo "  current:  $expected" >&2
  die "run scripts/build_web_ui.sh and land $FW_ASSETS_DIR through a PR"
fi

INDEX_GZ="$FW_ASSETS_DIR/index.html.gz"
[[ -f "$INDEX_GZ" ]] || die "missing mirrored index: $INDEX_GZ"

asset_refs="$(gunzip -c "$INDEX_GZ" | grep -oE '/assets/[^"]+' || true)"
if [[ -z "$asset_refs" ]]; then
  die "mirrored index references no /assets/ file — build_web_ui.sh output changed shape"
fi

while IFS= read -r ref; do
  [[ "$ref" == *"?v="* ]] && continue
  echo "verify_web_mirror_fresh: unstamped asset reference in the mirrored index:" >&2
  echo "  $ref" >&2
  die "assets are served immutable and MUST carry ?v=<hash> — check the sed in scripts/build_web_ui.sh"
done <<<"$asset_refs"

echo "verify_web_mirror_fresh: OK (the embedded bundle matches web/app, ?v= stamped)."
