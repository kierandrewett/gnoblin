#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
testdir="$(mktemp -d /tmp/gnoblin-background-test.XXXXXX)"
trap 'rm -rf "$testdir"' EXIT
for name in background-effect layer-shell; do
    if [[ "$name" == background-effect ]]; then
        # The repo does not carry this XML. Use the copy from the wayland-protocols package.
        xml="$root/src/protocols/background-effect/ext-background-effect-v1.xml"
        if [[ ! -f "$xml" ]]; then
            protocols_dir="$(pkg-config --variable=pkgdatadir wayland-protocols 2>/dev/null || true)"
            xml="$protocols_dir/staging/ext-background-effect/ext-background-effect-v1.xml"
        fi
    else xml="$root/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"; fi
    [[ -f "$xml" ]] || { echo "missing protocol XML for $name: $xml" >&2; exit 1; }
    wayland-scanner client-header "$xml" "$testdir/$name-client.h"
    wayland-scanner private-code "$xml" "$testdir/$name.c"
done
wayland-scanner private-code /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml "$testdir/xdg.c"
compiler_flags="$(pkg-config --cflags --libs wayland-client)" || exit 1
read -r -a compiler_args <<<"$compiler_flags"
cc -std=c11 -Wall -Wextra -Wno-unused-parameter "$root/tests/background-effect-client.c" "$testdir/"*.c \
    -I"$testdir" "${compiler_args[@]}" -o "$testdir/client"
if [[ "${GNOBLIN_ACTIVE_MODE:-gnoblin}" != gnoblin ]]; then
    "$testdir/client" stock
else
    "$testdir/client" duplicate
    "$testdir/client" dead
    python3 "$root/tests/test-background-effect.py" "$testdir/client"
fi
