#!/usr/bin/env bash
# Host: the idle service comes back after it is killed. Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/idle-restart-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-idle-restart.sh"
    # The guest keeps a system-wide copy of the unit from its setup, and sync-prefix does not update it. Install the unit
    # from the built prefix, so that the test runs against the unit under test. A source install links the unit instead.
    scripts/qemu-e2e guest "$run" -- sudo install -m644 "$prefix/lib/systemd/user/gnoblin-idle.service" \
        /usr/lib/systemd/user/gnoblin-idle.service
    scripts/qemu-e2e guest "$run" -- bash -c 'XDG_RUNTIME_DIR=/run/user/$(id -u); export XDG_RUNTIME_DIR; systemctl --user daemon-reload; systemctl --user restart gnoblin-idle.service'
    sleep 2
    scripts/qemu-e2e guest "$run" -- bash -s <"$here/guest-idle-restart.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
