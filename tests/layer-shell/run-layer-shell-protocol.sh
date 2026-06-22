#!/usr/bin/env bash
# Exercise wlr-layer-shell protocol invariants against a real gnoblin-shell.
#
# The probe includes both valid behaviour (map/remap, popup handoff) and client
# error cases. For error cases, success means the client receives the expected
# protocol error and the compositor stays alive.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SRC="$ROOT/scripts/layer-shell-probe.c"
LS_XML="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
XDG_XML="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }

BUILD="$(mktemp -d /tmp/lsproto.XXXXXX)"
trap 'rm -rf "$BUILD"' EXIT

wayland-scanner client-header "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-client-protocol.h" || exit 1
wayland-scanner private-code "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" || exit 1
wayland-scanner client-header "$XDG_XML" "$BUILD/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$XDG_XML" "$BUILD/xdg-shell-protocol.c" || exit 1

cc "$SRC" \
   "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" \
   "$BUILD/xdg-shell-protocol.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) \
   -o "$BUILD/layer-shell-probe" || {
  echo "FAIL: could not compile layer-shell protocol probe"; exit 1; }

modes=(
  map
  background
  popup
  invalid-size
  invalid-anchor
  invalid-keyboard
  invalid-exclusive-edge
  buffer-before-configure
  ack-before-configure
  bad-serial-high
  unknown-serial-low
  commit-after-later-configure
  state-change-configure
  null-before-first-map-resets
  invalid-layer
  pending-buffer-before-get-layer-surface
  committed-before-get-layer-surface
)

for mode in "${modes[@]}"; do
  echo "== layer-shell protocol probe: $mode =="
  out="$(GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/scripts/devkit-harness.py" run "$BUILD/layer-shell-probe" "$mode" 2>&1)"
  rc=$?
  echo "$out" | grep -vE "vfs|fusermount|gvfs|can't be made"

  if echo "$out" | grep -q "COMPOSITOR CRASHED"; then
    echo "FAIL: compositor crashed during probe mode '$mode'"
    exit 1
  fi
  if ! echo "$out" | grep -q "COMPOSITOR SURVIVED"; then
    echo "FAIL: harness did not report compositor survival for '$mode'"
    exit 1
  fi
  if [ "$rc" -ne 0 ]; then
    echo "FAIL: layer-shell protocol probe '$mode' exited $rc"
    exit 1
  fi
done

echo "PASS: layer-shell protocol invariants hold"
