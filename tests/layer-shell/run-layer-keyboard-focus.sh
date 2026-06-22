#!/usr/bin/env bash
# Regression: keyboard_interactivity=on_demand layer surfaces must be focusable.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SRC="$ROOT/tests/layer-shell/layer-keyboard-focus-client.c"
LS_XML="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
XDG_XML="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: gst-launch-1.0 not installed (needed for RemoteDesktop pointer injection)" >&2; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

BUILD="$(mktemp -d /tmp/lskeyfocus.XXXXXX)"
trap 'rm -rf "$BUILD"' EXIT

wayland-scanner client-header "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-client-protocol.h" || exit 1
wayland-scanner private-code "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" || exit 1
wayland-scanner client-header "$XDG_XML" "$BUILD/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$XDG_XML" "$BUILD/xdg-shell-protocol.c" || exit 1

cc "$SRC" \
   "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" \
   "$BUILD/xdg-shell-protocol.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) \
   -o "$BUILD/layer-keyboard-focus-client" || {
  echo "FAIL: could not compile layer keyboard focus client"; exit 1; }

echo "== on_demand layer-shell surfaces must receive keyboard focus =="
GNOBLIN_PREFIX="$PREFIX" \
GNOBLIN_LAYER_KEYBOARD_FOCUS_CLIENT="$BUILD/layer-keyboard-focus-client" \
GNOBLIN_LAYER_KEYBOARD_MODE=on_demand \
  python3 "$ROOT/tests/layer-shell/layer-keyboard-focus-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made" || exit "${PIPESTATUS[0]}"

echo "== exclusive layer-shell surfaces must receive keyboard focus on map =="
GNOBLIN_PREFIX="$PREFIX" \
GNOBLIN_LAYER_KEYBOARD_FOCUS_CLIENT="$BUILD/layer-keyboard-focus-client" \
GNOBLIN_LAYER_KEYBOARD_MODE=exclusive \
  python3 "$ROOT/tests/layer-shell/layer-keyboard-focus-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
