#!/usr/bin/env bash
# Host: blur rule check. Prints PASS or FAIL and saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/blur-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-blur.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/configs/99-test-blur.lua" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-blur.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
