#!/usr/bin/env bash
# Host: the unfocused-window dimming rule must follow focus. Prints PASS or FAIL and saves the output.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/unfocused-$(date -u +%Y%m%dT%H%M%SZ).txt"
guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/$1"; }
{
    echo "command: tests/qemu-e2e/run-unfocused.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/configs/99-test-unfocused.lua" luna@127.0.0.1:/tmp/
    setup="$(guest guest-unfocused-setup.sh)"
    echo "$setup"
    read -r _ _ ax ay < <(echo "$setup" | grep "CLICK dim-a")
    echo "== click dim-a at $ax $ay"
    scripts/qemu-e2e guest "$run" -- env DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus python3 - "$ax" "$ay" < "$here/guest-click.py"
    reading="$(guest guest-unfocused-read.sh)"
    echo "$reading"
    a_focus="$(echo "$reading" | grep "RESULT dim-a" | awk '{print $4}')"
    a_red="$(echo "$reading" | grep "RESULT dim-a" | awk '{print $6}')"
    b_red="$(echo "$reading" | grep "RESULT dim-b" | awk '{print $6}')"
    if [ "$a_focus" = "True" ] && [ "$a_red" -lt 80 ] && [ "$b_red" -gt 100 ]; then
        echo "PASS the focused window is opaque and the unfocused one is dimmed (a=$a_red b=$b_red)"
    else
        echo "FAIL expected dim-a focused and opaque, dim-b dimmed (focused=$a_focus a=$a_red b=$b_red)"
    fi
} 2>&1 | tee "$out"
echo "saved: $out"
