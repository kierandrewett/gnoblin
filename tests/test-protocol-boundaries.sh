#!/usr/bin/env bash
# Build and run black-box clients against Gnoblin-owned Wayland protocols.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
mkdir -p "$ROOT/build/tmp"
TMP="$(mktemp -d "$ROOT/build/tmp/protocol-boundaries.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

protocols=(
    foreign-toplevel-management/wlr-foreign-toplevel-management-unstable-v1.xml
    window-frame/kde-server-decoration.xml
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

compiler_flags="$(pkg-config --cflags --libs wayland-client)" || exit 1
read -r -a compiler_args <<<"$compiler_flags"
cc -std=c11 -Wall -Wextra -Werror \
    -I"$TMP" \
    "$ROOT/tests/protocol-boundary-client.c" \
    "${sources[@]}" \
    "${compiler_args[@]}" \
    -o "$TMP/protocol-boundary-client"

GNOBLIN_STATE_DIR="$TMP/state" \
    GNOBLIN_PREFIX="$PREFIX" \
    GNOBLIN_RUNTIME_BIN="$PREFIX/bin/gnoblin" \
    GNOBLIN_DEVKIT_CTL="$PREFIX/bin/gnoblinctl" \
    GNOBLIN_DEVKIT_EXEC="$TMP/protocol-boundary-client" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh"
