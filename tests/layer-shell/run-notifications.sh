#!/usr/bin/env bash
# Regression: the notification pipeline (gnoblin-notifyd daemon + popup + history)
# works. Drives a headless gnoblin-shell via the harness, sends a notification,
# and pixel-checks the popup and quick-settings history. Needs a dev build +
# grim + notify-send + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v notify-send >/dev/null || { echo "SKIP: no notify-send"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== notification popup + quick-settings history must render via notifyd (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/notifications-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
