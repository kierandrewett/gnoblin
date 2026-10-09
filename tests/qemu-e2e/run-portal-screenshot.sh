#!/usr/bin/env bash
# Host: the portal asks before it takes a screenshot, from the first request of a login. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/portal-screenshot-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-portal-screenshot.sh"
    # A fresh login, so that the first screenshot request of the test is the first one of the session.
    scripts/qemu-e2e guest "$run" -- sudo systemctl restart gdm
    sleep 30
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <"$here/guest-portal-screenshot.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
