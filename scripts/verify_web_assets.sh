#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

APP_DIR="$ROOT/web/app"
FW_ASSETS_DIR="$ROOT/crates/dali2rust-firmware/assets/web"
MANIFEST_RS="$ROOT/crates/dali2rust-firmware/src/web_assets.rs"

die() {
  echo "verify_web_assets: $*" >&2
  exit 1
}

[[ -d "$APP_DIR" ]] || die "missing web app dir: $APP_DIR"
[[ -f "$MANIFEST_RS" ]] || die "missing manifest: $MANIFEST_RS"

missing=()
while IFS= read -r rel; do
  [[ -f "$FW_ASSETS_DIR/$rel" ]] || missing+=("$rel")
done < <(grep -o 'asset_bytes!("[^"]*")' "$MANIFEST_RS" | sed 's/asset_bytes!("//; s/")//')

if ((${#missing[@]} > 0)); then
  echo "verify_web_assets: manifest embeds files absent from the mirror:" >&2
  printf '  %s\n' "${missing[@]}" >&2
  die "run scripts/build_web_ui.sh and commit $FW_ASSETS_DIR"
fi

if [[ -d "$APP_DIR/node_modules" ]]; then
  (cd "$APP_DIR" && npx --no-install tsc -b --noEmit 2>/dev/null || npx --no-install tsc -b) \
    || die "tsc -b failed in web/app"
  (cd "$APP_DIR" && npm test) || die "frontend tests failed in web/app"
  echo "verify_web_assets: OK (manifest, tsc and tests clean)."
else
  if [[ "${DALI2RUST_SKIP_TSC:-0}" != "1" ]]; then
    die "web/app/node_modules is missing, so nothing type-checked the web UI. \
Run 'npm ci' in web/app, or set DALI2RUST_SKIP_TSC=1 to say out loud that this \
run does not check it."
  fi
  echo "verify_web_assets: OK (manifest clean; tsc/tests SKIPPED by DALI2RUST_SKIP_TSC=1)."
fi
