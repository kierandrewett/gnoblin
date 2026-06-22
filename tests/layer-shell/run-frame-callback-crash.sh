#!/usr/bin/env bash
# Regression test for the layer-shell frame-callback assertion crash.
#
# History: gnoblin's mutter aborted with
#   meta-wayland-surface.c:1055: assertion failed:
#     (wl_list_empty (&state->frame_callback_list))
# whenever a layer-shell client attached a buffer + a frame callback on a commit
# that the layer-shell role rejected via an early return (e.g. "buffer before
# ack_configure", which fires during a configure storm when the monitor appears
# late in the devkit). The early returns bypassed the parent apply_state that
# consumes frame callbacks, so the generic surface code asserted and SIGABRTed.
#
# This test builds a deliberately-misbehaving layer-shell client that attaches a
# buffer + frame callback WITHOUT ack_configure, runs it against a real headless
# gnoblin-shell via the devkit harness, and asserts the compositor SURVIVES
# (sends the client a protocol error instead of aborting).
#
# Needs a dev build in ./install (run `just dev`). Self-contained: generates the
# protocol glue with wayland-scanner and compiles the client into a temp dir.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SRC="$ROOT/tests/layer-shell/frame-callback-crash.c"
LS_XML="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
XDG_XML="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }

BUILD="$(mktemp -d /tmp/lsfc.XXXXXX)"
trap 'rm -rf "$BUILD"' EXIT

wayland-scanner client-header "$LS_XML"  "$BUILD/wlr-layer-shell.h" || exit 1
wayland-scanner private-code  "$LS_XML"  "$BUILD/wlr-layer-shell.c" || exit 1
wayland-scanner private-code  "$XDG_XML" "$BUILD/xdg-shell.c"       || exit 1

cc "$SRC" "$BUILD/wlr-layer-shell.c" "$BUILD/xdg-shell.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) -o "$BUILD/adv" || {
  echo "FAIL: could not compile adversarial client"; exit 1; }

echo "== running adversarial layer-shell client against gnoblin-shell =="
out="$(GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/scripts/devkit-harness.py" run "$BUILD/adv" 2>&1)"
echo "$out" | grep -vE "vfs|fusermount|gvfs|can't be made"

if echo "$out" | grep -q "COMPOSITOR CRASHED"; then
  echo "FAIL: compositor aborted on the frame-callback path (regression)"
  exit 1
fi
if echo "$out" | grep -q "COMPOSITOR SURVIVED"; then
  echo "PASS: compositor rejected the bad commit and stayed alive"
  exit 0
fi
echo "FAIL: inconclusive (harness did not report survival)"
exit 1
