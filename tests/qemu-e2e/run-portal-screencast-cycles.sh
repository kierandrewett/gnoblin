#!/usr/bin/env bash
# Host: every portal screen cast cycle opens the consent dialog, drives it to Share, reads frames and closes, with no growth. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
cycles="${CYCLES:-10}"
out="$run/portal-screencast-cycles-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: CYCLES=$cycles tests/qemu-e2e/run-portal-screencast-cycles.sh"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" CYCLES="$cycles" bash -s <"$here/guest-portal-screencast-cycles.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
