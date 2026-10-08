#!/usr/bin/env bash
# Host: the session recovers when the session supervisor is killed. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/supervisor-restart-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-supervisor-restart.sh"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <"$here/guest-supervisor-restart.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
