#!/usr/bin/env bash
# Host: X11 app size at a fractional monitor scale. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/x11-fractional-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-x11-fractional.sh"
    status=0
    echo "--- scale 1.25 at the preferred mode"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <"$here/guest-x11-fractional.sh" || status=1
    echo "--- scale 1.5 at 1920x1080"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" GNOBLIN_TEST_SCALE=1.5 \
        GNOBLIN_TEST_MODE=1920x1080@60.000 bash -s <"$here/guest-x11-fractional.sh" || status=1
    echo "--- scale 2 at 1920x1080"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" GNOBLIN_TEST_SCALE=2 \
        GNOBLIN_TEST_MODE=1920x1080@60.000 bash -s <"$here/guest-x11-fractional.sh" || status=1
    echo "overall: $([ "$status" -eq 0 ] && echo pass || echo FAIL)"
} 2>&1 | tee "$out"
echo "saved: $out"
