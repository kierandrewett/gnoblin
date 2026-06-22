#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
KDE_XML="$ROOT/src/protocols/kde-appmenu/kde-appmenu.xml"
XDG_XML="$(pkg-config --variable=pkgdatadir wayland-protocols 2>/dev/null)/stable/xdg-shell/xdg-shell.xml"
BUILD="$(mktemp -d /tmp/kdeappmenu.XXXXXX)"
PROBE="$BUILD/kde-appmenu-probe"
trap 'rm -rf "$BUILD"' EXIT

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }
[ -f "$KDE_XML" ] || { echo "SKIP: kde-appmenu XML not present"; exit 0; }
[ -f "$XDG_XML" ] || { echo "SKIP: xdg-shell XML not present"; exit 0; }

wayland-scanner client-header "$KDE_XML" "$BUILD/kde-appmenu-client-protocol.h" || exit 1
wayland-scanner private-code "$KDE_XML" "$BUILD/kde-appmenu-protocol.c" || exit 1
wayland-scanner client-header "$XDG_XML" "$BUILD/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$XDG_XML" "$BUILD/xdg-shell-protocol.c" || exit 1

cc "$ROOT/tests/layer-shell/kde-appmenu-probe.c" \
   "$BUILD/kde-appmenu-protocol.c" \
   "$BUILD/xdg-shell-protocol.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) \
   -o "$PROBE" || { echo "FAIL: could not build KDE appmenu probe"; exit 1; }

echo "== KDE appmenu protocol must honor topbar.appmenu-backend =="
GNOBLIN_PREFIX="$PREFIX" KDE_APPMENU_PROBE="$PROBE" \
  python3 "$ROOT/tests/layer-shell/kde-appmenu-backend-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
