#!/usr/bin/env bash
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)/web/app"

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
