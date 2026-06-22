#!/usr/bin/env bash
# Regression: gnoblin-wallpaper renders the configured image as the desktop
# background. Drives a headless gnoblin-shell via the harness with a distinctive
# wallpaper and pixel-checks the desktop. Needs a dev build + grim + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-wallpaper" ] || { echo "SKIP: gnoblin-wallpaper not built"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== gnoblin-wallpaper must render the configured image (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/wallpaper-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
