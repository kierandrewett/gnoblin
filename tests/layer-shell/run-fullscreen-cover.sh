#!/usr/bin/env bash
# Regression: a fullscreen window must cover the topbar/dock (wlr `top` panels),
# not be obscured by them. Drives a headless gnoblin-shell via the harness,
# fullscreens a real window, and pixel-checks that the top strip is covered.
# Needs a dev build + grim + foot + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== fullscreen must cover the topbar/dock panels (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/fullscreen-cover-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
