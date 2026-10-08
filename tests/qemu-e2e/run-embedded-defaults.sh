#!/usr/bin/env bash
# Host: a login with no user config loads the embedded default tree, and gnoblinctl config restore-default installs a
# fresh copy of it. Restarts the guest session twice (once with init.lua hidden, once to restore). Saves the output in
# the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/embedded-defaults-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0

guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s -- "$@" < "$here/guest-embedded-defaults.sh"; }
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
    echo "command: tests/qemu-e2e/run-embedded-defaults.sh"
    guest hide
    restart
    result="$(guest read)"
    guest unhide
    restart
    echo "--- guest report (no user config)"
    printf '%s\n' "$result"
    echo "---"
    check "the session runs with no user config" "$result" 'status="state":"running"'
    check "it did not fall back" "$result" "marker=none"
    check "the user init.lua is absent" "$result" "init_present=no"
    check "a server-decorated window gets the native frame" "$result" "frame=702x433"
    restored="$(guest restore)"
    echo "--- guest report (restore-default)"
    printf '%s\n' "$restored"
    echo "---"
    check "restore-default installs an init.lua" "$restored" "new_init=yes"
    check "the new folder has the starter shortcuts" "$restored" "new_has_shortcuts=yes"
    check "the new folder has the current starter bindings" "$restored" "new_has_switch_binding=2"
    check "the new folder holds none of the old test files" "$restored" "new_has_no_test_files=0"
    check "the previous folder is kept as a backup" "$restored" "backup_has_old_files=yes"
    check "the backup keeps the user's other files" "$restored" "backup_kept_bingux=yes"
    check "the session runs after the restore" "$restored" 'status_after="state":"running"'
    check "the original folder was put back" "$restored" "restored_init=yes"
    echo "failures: $fail"
} 2>&1 | tee "$out"
echo "saved: $out"
