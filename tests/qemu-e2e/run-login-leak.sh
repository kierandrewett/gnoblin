#!/usr/bin/env bash
# Host: leftover processes after repeated logins (GitHub: accessibility registry leak). Saves the output in the run directory.
#
# Restarts the display manager GNOBLIN_TEST_LOGINS times (default 3), which ends the session and starts a new one. After
# each login the number of at-spi2-registryd processes must stay at 2 or fewer. It was one more per login before.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
logins="${GNOBLIN_TEST_LOGINS:-3}"
out="$run/login-leak-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-login-leak.sh"
    fail=0
    for n in $(seq 1 "$logins"); do
        scripts/qemu-e2e guest "$run" -- sudo systemctl restart gdm
        sleep 30
        count="$(scripts/qemu-e2e guest "$run" -- bash -s <"$here/guest-login-leak.sh" | sed -n 's/^registry_processes=//p')"
        echo "login $n: $count registry processes"
        if [ "${count:-99}" -le 2 ]; then
            echo "PASS login $n leaves no extra accessibility registry processes"
        else
            echo "FAIL login $n left $count accessibility registry processes"
            fail=$((fail + 1))
        fi
    done
    echo "failures: $fail"
    exit "$fail"
} 2>&1 | tee "$out"
echo "saved: $out"
