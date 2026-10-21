#!/usr/bin/env bash
# Host: repeated portal screen casts leave no stream node, session object, open file or memory behind. Saves the output in
# the run directory. CYCLES (default 20) sets how many times the portal flow runs.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
cycles="${CYCLES:-20}"
out="$run/portal-screencast-cycles-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-portal-screencast-cycles.sh"
    echo "cycles: $cycles"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" CYCLES="$cycles" bash -s <"$here/guest-portal-screencast-cycles.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
