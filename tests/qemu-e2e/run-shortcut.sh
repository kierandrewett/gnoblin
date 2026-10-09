#!/usr/bin/env bash
# Host: dynamic shortcut end-to-end check. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/shortcut-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-shortcut.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here"/configs/99-test-shortcut.lua luna@127.0.0.1:/tmp/
    echo "== bind"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-shortcut-install.sh"
    echo "== send Super+Alt+F9"
    python3 -I "$here/send-combo.py" "$run/qmp.sock" meta_l alt f9
    sleep 1
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-shortcut-read.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
