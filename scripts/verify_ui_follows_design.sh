#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

VISUAL_PATHS=(web/app/src/screens web/app/src/components web/app/src/app.css)
BASE="${1:-HEAD}"

visual="$( { git diff --name-only "$BASE" -- "${VISUAL_PATHS[@]}" 2>/dev/null || true
             git diff --cached --name-only -- "${VISUAL_PATHS[@]}" 2>/dev/null || true; } | sort -u )"

if pushed_out="$(bash scripts/verify_design_system_pushed.sh 2>&1)"; then
  echo "$pushed_out"
  if [[ -n "$visual" ]]; then
    echo "verify_ui_follows_design: OK (design system is in sync; web/app may implement it)"
  else
    echo "verify_ui_follows_design: OK (no visual change in web/app)"
  fi
  exit 0
fi

echo "$pushed_out" >&2
if [[ -z "$visual" ]]; then
  echo "verify_ui_follows_design: design system is out of sync (no web/app change yet)" >&2
  exit 1
fi
cat >&2 <<MSG

verify_ui_follows_design: web/app changed visually while the design system is
out of sync with the Claude Design project:

$(sed 's/^/  /' <<<"$visual")

The UI is authored in the pane and web/app implements it. Sync the design system
first (change the card in Claude Design, pull it here), then land the screen.
MSG
exit 1
