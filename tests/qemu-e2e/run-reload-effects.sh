#!/usr/bin/env bash
# Host: autostart and focus_mode reload checks. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/reload-effects-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-reload-effects.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/guest-click.py" "$here/guest-titlebar-drag.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-reload-effects.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
