#!/usr/bin/env bash
# Host: apps follow dark mode and high contrast live through the portal. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/appearance-settings-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-appearance-settings.sh"
    scripts/qemu-e2e guest "$run" -- bash -s <"$here/guest-appearance-settings.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
