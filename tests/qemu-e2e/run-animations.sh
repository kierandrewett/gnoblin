#!/usr/bin/env bash
# Host: check whether open and dialog-open animations run. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/animations-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-animations.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "$here/configs/99-test-animations.lua" "$here/dialog-test.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-animations.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
