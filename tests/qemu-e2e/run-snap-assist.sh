#!/usr/bin/env bash
# Host: Bingux Super + left-drag snap-assist check. Saves output in the QEMU run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
if [[ -n "${GNOBLIN_PREFIX:-}" ]]; then
    prefix="$GNOBLIN_PREFIX"
elif [[ -r "$run/synced-prefix" ]]; then
    prefix="$(cat "$run/synced-prefix")"
else
    prefix="$root/build/validation-install"
fi
port="$(cat "$run/ssh-port")"
out="$run/snap-assist-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-snap-assist.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/guest-resize-drag.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <"$here/guest-snap-assist.sh"
} 2>&1 | tee "$out"
echo "saved: $out"
