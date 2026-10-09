#!/usr/bin/env bash
# Host: Super+Space and Shift+Super+Space switch the input source. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/input-source-shortcut-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-input-source-shortcut.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "$root/src/data/default-config/config/40-shortcuts.lua" luna@127.0.0.1:/tmp/40-shortcuts.lua
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-input-source-shortcut.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
