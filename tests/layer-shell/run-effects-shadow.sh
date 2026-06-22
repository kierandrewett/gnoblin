#!/usr/bin/env bash
# Regression: the compositor draws a drop shadow around windows. Drives a headless
# gnoblin-shell via the harness, spawns a floating window, and pixel-checks the
# shadow below it. Needs a dev build + grim + foot + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== windows must have a drop shadow (effects layer, headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/effects-shadow-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
