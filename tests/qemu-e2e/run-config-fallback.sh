#!/usr/bin/env bash
# Host: a config that cannot be loaded at all (a Lua syntax error) falls back, keeps the session running, and says why
# in the marker and the log. A config with one invalid setting is covered by run-partial-recovery.sh.
# Restarts the guest session twice (once with the bad config, once to restore). Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/config-fallback-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0

guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s -- "$@" < "$here/guest-config-fallback.sh"; }
restart() { scripts/qemu-e2e guest "$run" -- sudo systemctl restart gdm >/dev/null 2>&1; sleep 45; }
check() {
    # check NAME HAYSTACK NEEDLE
    if printf '%s' "$2" | grep -q -F -- "$3"; then
        echo "PASS $1"
    else
        echo "FAIL $1 (missing: $3)"
        fail=$((fail + 1))
    fi
}

{
    echo "command: tests/qemu-e2e/run-config-fallback.sh"
    guest install
    restart
    result="$(guest read)"
    guest clean
    restart
    check "the session keeps running with an invalid config" "$result" "status=\"state\":\"running\""
    check "the compositor stays up" "$result" "compositors=4"
    check "the marker names the fallback" "$result" "marker=last-good"
    check "the marker keeps the error" "$result" "99-test-config-fallback.lua"
    check "the session log names the choice and the file" "$result" "log=configuration fell back to the last good configuration:"
    echo "failures: $fail"
} 2>&1 | tee "$out"
echo "saved: $out"
