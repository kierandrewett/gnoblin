#!/usr/bin/env bash
# Host: location agent experiment. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/location-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-location.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "$here/configs/99-test-location.lua" "$here/geoclue-ask.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-location.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
