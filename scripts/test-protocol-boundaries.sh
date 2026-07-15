#!/usr/bin/env bash
# Build and run black-box clients against Gnoblin-owned Wayland protocols.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-protocol-boundaries.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

protocols=(
    foreign-toplevel-management/wlr-foreign-toplevel-management-unstable-v1.xml
    layer-shell/wlr-layer-shell-unstable-v1.xml
    screencopy/wlr-screencopy-unstable-v1.xml
)
sources=()
for protocol in "${protocols[@]}"; do
    xml="$ROOT/src/protocols/$protocol"
    base="$(basename "$xml" .xml)"
    wayland-scanner client-header "$xml" "$TMP/$base-client-protocol.h"
    wayland-scanner private-code "$xml" "$TMP/$base-protocol.c"
    sources+=("$TMP/$base-protocol.c")
done

# Layer-shell's generated type table references xdg_popup_interface.
wayland_protocols_dir="$(pkg-config --variable=pkgdatadir wayland-protocols)"
xdg_xml="$wayland_protocols_dir/stable/xdg-shell/xdg-shell.xml"
wayland-scanner client-header "$xdg_xml" "$TMP/xdg-shell-client-protocol.h"
wayland-scanner private-code "$xdg_xml" "$TMP/xdg-shell-protocol.c"
sources+=("$TMP/xdg-shell-protocol.c")

cc -std=c11 -Wall -Wextra -Werror \
    -I"$TMP" \
    "$ROOT/tests/protocol-boundary-client.c" \
    "${sources[@]}" \
    $(pkg-config --cflags --libs wayland-client) \
    -o "$TMP/protocol-boundary-client"

GNOBLIN_TEST_CLIENT="$TMP/protocol-boundary-client" \
    "$ROOT/scripts/run-gnome-shell.sh"
