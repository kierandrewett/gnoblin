#!/usr/bin/env bash
# Host: drag and drop of text between Wayland and X11 windows. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/drag-drop-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-drag-drop.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/guest-titlebar-drag.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-drag-drop.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
