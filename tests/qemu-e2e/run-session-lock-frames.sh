#!/usr/bin/env bash
# Host: a locker that waits for frame callbacks (like Qt Wayland) gets them while it holds the lock. Saves the output in
# the run directory. The locker is built here and copied into the guest, which has no compiler.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/session-lock-frames-$(date -u +%Y%m%dT%H%M%SZ).txt"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
xml="$root/src/protocols/session-lock/ext-session-lock-v1.xml"
wayland-scanner client-header "$xml" "$work/ext-session-lock-v1-client-protocol.h"
wayland-scanner private-code "$xml" "$work/ext-session-lock-v1-protocol.c"
cc -Wall -I"$work" -o "$work/session-lock-frame-client" "$here/session-lock-frame-client.c" \
    "$work/ext-session-lock-v1-protocol.c" -lwayland-client
{
    echo "command: tests/qemu-e2e/run-session-lock-frames.sh"
    {
        echo "base64 -d > /tmp/session-lock-frame-client <<'BIN'"
        base64 "$work/session-lock-frame-client"
        echo "BIN"
        cat "$here/guest-session-lock-frames.sh"
    } | scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s
} 2>&1 | tee "$out"
echo "saved: $out"
