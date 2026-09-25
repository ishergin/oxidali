#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

VISUAL_PATHS=(web/app/src/screens web/app/src/components web/app/src/app.css)
CARDS_DIR=web/design-system
PUSH_STAMP=web/design-system/.pushed.sha256
TRAILER="UI-Design: unchanged"

base_commit() {
  if [[ -n "${1:-}" ]]; then
    git rev-parse "$1"
  else
    git merge-base HEAD origin/main 2>/dev/null || git rev-parse HEAD
  fi
}

changed_under() {
  {
    git diff --name-only "$BASE" -- "$@"
    git ls-files --others --exclude-standard -- "$@"
  } | sort -u
}

BASE="$(base_commit "${1:-}")"
SINCE="since $(git rev-parse --short "$BASE")"

visual="$(changed_under "${VISUAL_PATHS[@]}")"
cards="$(changed_under "$CARDS_DIR" | grep -vxF "$PUSH_STAMP" || true)"

if [[ -z "$visual" ]]; then
  echo "verify_ui_follows_design: OK (no visual change in web/app $SINCE)"
  exit 0
fi
if [[ -n "$cards" ]]; then
  echo "verify_ui_follows_design: OK (web/app and its cards change together $SINCE)"
  exit 0
fi
if git log --format=%B "$BASE..HEAD" | grep -qxF "$TRAILER"; then
  echo "verify_ui_follows_design: OK (a commit declares '$TRAILER' $SINCE)"
  exit 0
fi

cat >&2 <<MSG
verify_ui_follows_design: web/app changed visually $SINCE, but no card in
$CARDS_DIR/ changed with it:

$(sed 's/^/  /' <<<"$visual")

Every screen and component has a card, and a change to how it looks changes the
card in the same pull request. If the change does not alter what the screen
shows (a refactor, a data fix), say so with the commit trailer
'$TRAILER'.
MSG
exit 1
