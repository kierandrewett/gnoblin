#!/usr/bin/env bash
# Regression: gnoblin-shell advertises its full wlr-/ext- protocol set. Builds the
# wl_globals probe, boots a headless gnoblin-shell via the harness, and asserts
# every expected global interface is in the registry. Needs a dev build.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
PROBE=/tmp/gnoblin-wl-globals

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
pkg-config --exists wayland-client 2>/dev/null || { echo "SKIP: no wayland-client dev"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

# Build the probe if missing or stale.
if [ ! -x "$PROBE" ] || [ "$ROOT/scripts/wl-globals.c" -nt "$PROBE" ]; then
  cc "$ROOT/scripts/wl-globals.c" $(pkg-config --cflags --libs wayland-client) -o "$PROBE" \
    || { echo "FAIL: could not build wl-globals probe"; exit 1; }
fi

echo "== gnoblin-shell must advertise its full protocol set (headless) =="
GNOBLIN_PREFIX="$PREFIX" GNOBLIN_WL_GLOBALS="$PROBE" \
  python3 "$ROOT/tests/layer-shell/protocols-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
