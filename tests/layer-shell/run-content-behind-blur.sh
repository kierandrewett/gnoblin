#!/usr/bin/env bash
# Regression: the compositor blur frosts the WINDOW stacked behind a translucent
# blurred surface (real content-behind blur), not just the wallpaper. Drives a
# headless gnoblin-shell via the harness. Needs a dev build + foot.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL, numpy" 2>/dev/null || { echo "SKIP: needs python gi/PIL/numpy"; exit 0; }

echo "== content-behind blur frosts the window underneath (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/content-behind-blur-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
