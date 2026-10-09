#!/usr/bin/env bash
# Host: window churn memory check. Prints PASS or FAIL and saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/memory-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-memory.sh"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" \
        GNOBLIN_MEMORY_SECONDS="${GNOBLIN_MEMORY_SECONDS:-90}" \
        GNOBLIN_MEMORY_LIMIT_MB="${GNOBLIN_MEMORY_LIMIT_MB:-40}" \
        GNOBLIN_MEMORY_CLICKS="${GNOBLIN_MEMORY_CLICKS:-1}" bash -s < "$here/guest-memory.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
