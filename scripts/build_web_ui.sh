#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/web/app"
DIST_DIR="$APP_DIR/dist"
OUT_DIR="$APP_DIR/dist-gz"
FW_ASSETS_DIR="$ROOT/crates/dali2rust-firmware/assets/web"

cd "$APP_DIR"
npm run build

BUNDLE_HASH="$(cat "$DIST_DIR"/assets/* | shasum -a 256 | cut -c1-12)"
sed -i '' -E "s#(/assets/[A-Za-z0-9._-]+)\"#\1?v=$BUNDLE_HASH\"#g" "$DIST_DIR/index.html"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

(cd "$DIST_DIR" && find . -type f ! -name '*.map' -print0) | while IFS= read -r -d '' rel; do
  rel="${rel#./}"
  mkdir -p "$OUT_DIR/$(dirname "$rel")"
  gzip -9 -n -c "$DIST_DIR/$rel" > "$OUT_DIR/$rel.gz"
done

echo "build_web_ui: gzipped assets in $OUT_DIR"
(cd "$OUT_DIR" && find . -type f -exec ls -la {} \; | awk '{print $5, $9}')

MANIFEST_FILES=(index.html.gz favicon.svg.gz assets/app.js.gz assets/app.css.gz dali-products.json.gz)

rm -rf "$FW_ASSETS_DIR"
for rel in "${MANIFEST_FILES[@]}"; do
  if [[ ! -f "$OUT_DIR/$rel" ]]; then
    echo "build_web_ui: FATAL — manifest file '$rel' missing from $OUT_DIR" >&2
    exit 1
  fi
  mkdir -p "$FW_ASSETS_DIR/$(dirname "$rel")"
  cp "$OUT_DIR/$rel" "$FW_ASSETS_DIR/$rel"
done

UNMIRRORED="$(cd "$OUT_DIR" && find . -type f | sed 's#^\./##' \
  | grep -vxF "$(printf '%s\n' "${MANIFEST_FILES[@]}")" || true)"
if [[ -n "$UNMIRRORED" ]]; then
  echo "build_web_ui: WARNING — built but NOT embedded (add to web_assets.rs):" >&2
  echo "$UNMIRRORED" | sed 's/^/  /' >&2
fi

(
  cd "$APP_DIR"
  find src public -type f -print0 2>/dev/null
  printf '%s\0' index.html package.json package-lock.json vite.config.ts
  printf '%s\0' tsconfig.json tsconfig.app.json tsconfig.node.json
) | LC_ALL=C sort -z | while IFS= read -r -d '' rel; do
  [[ -f "$APP_DIR/$rel" ]] || continue
  printf '%s ' "$rel"
  shasum -a 256 "$APP_DIR/$rel" | cut -d' ' -f1
done | shasum -a 256 | cut -d' ' -f1 > "$FW_ASSETS_DIR/.sources.sha256"

echo "build_web_ui: firmware mirror refreshed in $FW_ASSETS_DIR"
echo "build_web_ui: source stamp $(cat "$FW_ASSETS_DIR/.sources.sha256")"
echo "build_web_ui: embedded total $(find "$FW_ASSETS_DIR" -name '*.gz' -exec cat {} + \
  | wc -c | tr -d ' ') B"
