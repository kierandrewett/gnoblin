#!/usr/bin/env bash
# Host: diagnostic heap profile of the compositor under window churn. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/heaptrack-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-heaptrack.sh"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" \
        GNOBLIN_HEAPTRACK_CYCLES="${GNOBLIN_HEAPTRACK_CYCLES:-60}" bash -s < "$here/guest-heaptrack.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
