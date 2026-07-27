#!/usr/bin/env bash
# Layer-shell chrome latency: how long a bar takes to get its first pixel up.
#
# gnoblin draws no chrome of its own, so this is the number that decides
# whether the desktop feels responsive -- more so than boot time, because it is
# paid every time a bar, dock or popup surface appears. Nothing measured it
# before this script.
#
# Builds tests/layer-shell-latency-client.c against the in-tree layer-shell
# XML, runs it inside a headless gnoblin session, and reports process start ->
# first frame. See the client's header for what each mark means.
#
# Env: GNOBLIN_PREFIX (default ./install), LAYER_BUDGET_MS (default 0 = report
#      only, no budget), SETTLE (passed through).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-layer-latency.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

BUDGET_MS="${LAYER_BUDGET_MS:-0}"

# Same generation dance as test-protocol-boundaries.sh: layer-shell's generated
# type table references xdg_popup_interface, so xdg-shell has to come along.
xml="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
wayland-scanner client-header "$xml" "$TMP/wlr-layer-shell-unstable-v1-client-protocol.h"
wayland-scanner private-code  "$xml" "$TMP/wlr-layer-shell-unstable-v1-protocol.c"

wayland_protocols_dir="$(pkg-config --variable=pkgdatadir wayland-protocols)"
xdg_xml="$wayland_protocols_dir/stable/xdg-shell/xdg-shell.xml"
wayland-scanner client-header "$xdg_xml" "$TMP/xdg-shell-client-protocol.h"
wayland-scanner private-code  "$xdg_xml" "$TMP/xdg-shell-protocol.c"

cc -std=c11 -Wall -Wextra -Werror \
    -I"$TMP" \
    "$ROOT/tests/layer-shell-latency-client.c" \
    "$TMP/wlr-layer-shell-unstable-v1-protocol.c" \
    "$TMP/xdg-shell-protocol.c" \
    $(pkg-config --cflags --libs wayland-client) \
    -o "$TMP/layer-latency-client"

echo "== layer-shell chrome latency (headless, ${GNOBLIN_PREFIX:-$ROOT/install}) =="
OUT="$TMP/run.log"
GNOBLIN_TEST_CLIENT="$TMP/layer-latency-client" \
    "$ROOT/scripts/run-gnome-shell.sh" >"$OUT" 2>&1 || true

if ! grep -q "LAYER_SHELL_LATENCY" "$OUT"; then
    echo "FAIL: client did not report a measurement" >&2
    grep -iE "layer-latency:|RESULT:" "$OUT" >&2 | head -5 || true
    exit 1
fi

grep -E "^   (connect|globals|configure|frame) " "$OUT" || true
read -r _ c_us g_us cfg_us frame_us < <(grep "LAYER_SHELL_LATENCY" "$OUT" | tail -1)
frame_ms=$(( frame_us / 1000 ))
echo "   -> first pixel at ${frame_ms} ms from client start"

if [ "$BUDGET_MS" -gt 0 ] && [ "$frame_ms" -gt "$BUDGET_MS" ]; then
    echo "FAIL: layer-shell first frame ${frame_ms} ms exceeds budget ${BUDGET_MS} ms" >&2
    exit 1
fi
echo "PASS: layer-shell latency measured"
