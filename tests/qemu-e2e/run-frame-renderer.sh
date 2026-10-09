#!/usr/bin/env bash
# Host: an external frame renderer registered in frame_renderers and selected by a window rule draws the titlebar.
# Builds the sample cairo renderer, copies it into the guest, and compares the titlebar with the native frame. Saves the
# output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/frame-renderer-$(date -u +%Y%m%dT%H%M%SZ).txt"

{
    echo "command: tests/qemu-e2e/run-frame-renderer.sh"
    GNOBLIN_FRAME_RENDERERS=cairo bash scripts/build-frame-renderers.sh "$root/build/frame-renderers"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "$root/build/frame-renderers/gnoblin-frame-cairo" luna@127.0.0.1:/tmp/gnoblin-frame-cairo
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-frame-renderer.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
