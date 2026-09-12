#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
testdir="$(mktemp -d /tmp/gnoblin-background-test.XXXXXX)"
trap 'rm -rf "$testdir"' EXIT
for name in background-effect layer-shell; do
    if [[ "$name" == background-effect ]]; then xml="$root/src/protocols/background-effect/ext-background-effect-v1.xml";
    else xml="$root/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"; fi
    wayland-scanner client-header "$xml" "$testdir/$name-client.h"
    wayland-scanner private-code "$xml" "$testdir/$name.c"
done
wayland-scanner private-code /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml "$testdir/xdg.c"
cc -std=c11 -Wall -Wextra -Wno-unused-parameter "$root/tests/background-effect-client.c" "$testdir/"*.c \
    -I"$testdir" $(pkg-config --cflags --libs wayland-client) -o "$testdir/client"
if [[ "${GNOBLIN_ACTIVE_MODE:-gnoblin}" != gnoblin ]]; then
    "$testdir/client" absent
else
    "$testdir/client" duplicate
    "$testdir/client" dead
    python3 "$root/scripts/test-background-effect.py" "$testdir/client"
fi
