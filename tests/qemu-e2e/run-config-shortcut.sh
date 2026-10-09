#!/usr/bin/env bash
# Host: config-declared command shortcut check. Saves the output in the run directory.
#
# gnoblin.configure {shortcuts = {...}} must bind a command, move it when the binding changes on reload, and drop it
# when the entry is disabled on reload. Keys are sent from the host through QMP, as in run-shortcut.sh.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/config-shortcut-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0

guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s -- "$@" < "$here/guest-config-shortcut.sh"; }
press() { python3 -I "$here/send-combo.py" "$run/qmp.sock" "$@" >/dev/null; }
check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

{
    echo "command: tests/qemu-e2e/run-config-shortcut.sh"
    guest install "<Super><Alt>F8" >/dev/null
    press meta_l alt f8
    check "a configured shortcut runs its command" "$(guest hit)" "hit"

    guest install "<Super><Alt>F7" >/dev/null
    press meta_l alt f8
    check "after the binding changes, the old keys do nothing" "$(guest hit)" "none"
    press meta_l alt f7
    check "after the binding changes, the new keys run the command" "$(guest hit)" "hit"

    guest disable >/dev/null
    press meta_l alt f7
    check "a disabled shortcut does nothing" "$(guest hit)" "none"

    guest clean >/dev/null
    echo "failures: $fail"
} 2>&1 | tee "$out"
echo "saved: $out"
