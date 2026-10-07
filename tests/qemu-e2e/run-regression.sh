#!/usr/bin/env bash
# Host side of the regression run. Copies the test configs, runs the guest script, saves the output.
# Run from anywhere. The guest must already run the build under test (scripts/qemu-e2e sync-prefix
# and a restart of the display manager).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/regression-$(date -u +%Y%m%dT%H%M%SZ).txt"

{
    echo "command: tests/qemu-e2e/run-regression.sh"
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "build: $(git rev-parse --short HEAD) with working tree changes"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "$here"/configs/99-test-capture.lua "$here"/configs/99-test-doc.lua "$here"/configs/99-test-prompts.lua "$here"/configs/99-test-rules.lua "$here"/configs/99-test-opacity.lua luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-regression.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
