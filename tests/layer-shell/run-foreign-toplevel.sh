#!/usr/bin/env bash
# Regression: ext-foreign-toplevel-list-v1 streams the window list (a taskbar/pager
# relies on it). Builds a probe client from the vendored protocol XML, boots a
# headless gnoblin-shell with a foot window, and asserts the probe sees foot.
# Needs a dev build + wayland-scanner + foot.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
XML="$ROOT/src/protocols/foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml"
BUILD="$(mktemp -d /tmp/ftprobe.XXXXXX)"
PROBE="$BUILD/probe"
trap 'rm -rf "$BUILD"' EXIT

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }
[ -f "$XML" ] || { echo "SKIP: protocol XML not present"; exit 0; }

wayland-scanner client-header "$XML" "$BUILD/ext-foreign-toplevel-list.h" || exit 1
wayland-scanner private-code  "$XML" "$BUILD/ext-foreign-toplevel-list.c" || exit 1
cc "$ROOT/tests/layer-shell/foreign-toplevel-probe.c" "$BUILD/ext-foreign-toplevel-list.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) -o "$PROBE" \
   || { echo "FAIL: could not build the foreign-toplevel probe"; exit 1; }

echo "== ext-foreign-toplevel-list must stream the window list (headless) =="
GNOBLIN_PREFIX="$PREFIX" FT_PROBE="$PROBE" \
  python3 "$ROOT/tests/layer-shell/foreign-toplevel-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
