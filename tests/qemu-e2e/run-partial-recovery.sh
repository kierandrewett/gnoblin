#!/usr/bin/env bash
# Host: one invalid setting, or one included file with a Lua error, at login must not cost the valid ones. The valid
# cursor size stays active, the invalid keybinding and the broken file are ignored and named in the session log and the
# notice, and the session does not fall back.
# Restarts the guest session twice (once with the bad config, once to restore). Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/partial-recovery-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0

guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s -- "$@" < "$here/guest-partial-recovery.sh"; }
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
    echo "command: tests/qemu-e2e/run-partial-recovery.sh"
    guest install
    restart
    result="$(guest read)"
    guest clean
    restart
    echo "--- guest report"
    printf '%s\n' "$result"
    echo "---"
    check "the session keeps running" "$result" "status=\"state\":\"running\""
    check "the compositor stays up" "$result" "compositors=4"
    check "the valid setting stays active" "$result" "cursor=\"size\":55"
    check "the log names the ignored keybinding" "$result" "log=ignored keybindings"
    check "the log gives the reason" "$result" "has no action in this Mutter build"
    check "the log names the skipped file" "$result" "file_log=ignored file"
    check "the skipped file is the syntax error one" "$result" "98-test-partial-recovery-syntax.lua"
    check "the notice lists what was ignored" "$result" "Some settings were ignored"
    check "the notice is not a fallback" "$result" "marker=notice"
    check "nothing fell back" "$result" "fellback=0"
    echo "failures: $fail"
} 2>&1 | tee "$out"
echo "saved: $out"
