#!/usr/bin/env bash
# Regression: background layer-shell wallpaper must not intercept app input.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-wallpaper" ] || { echo "SKIP: gnoblin-wallpaper not built"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0 (needed for pointer injection)"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

echo "== background layer must not intercept pointer input =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/background-layer-input-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
