#!/usr/bin/env bash
# Host: a background layer surface follows an output mode change. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/layer-output-resize-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-layer-output-resize.sh"
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <"$here/guest-layer-output-resize.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
