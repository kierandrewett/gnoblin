#!/usr/bin/env bash
# Host: pixel-check client-rendered panel opacity against compositor blur opacity.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
guest_artifacts="/tmp/gnoblin-blur-fade-$stamp"
out="$run/blur-fade-$stamp.txt"
{
    echo "command: tests/qemu-e2e/run-blur-fade.sh"
    scripts/qemu-e2e guest "$run" -- env \
        GNOBLIN_PREFIX="$prefix" \
        GNOBLIN_QS=/home/luna/.local/share/gnoblin/quickshell-fedora/bin/qs \
        GNOBLIN_BLUR_ARTIFACT_DIR="$guest_artifacts" \
        bash -s <"$here/guest-blur-fade.sh"
    scp -qr -P "$port" -o UserKnownHostsFile="$run/known_hosts" \
        "luna@127.0.0.1:$guest_artifacts" "$run/"
    echo "pixel captures: $run/$(basename "$guest_artifacts")"
} 2>&1 | tee "$out"
echo "saved: $out"
