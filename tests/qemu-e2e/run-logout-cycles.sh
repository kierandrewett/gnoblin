#!/usr/bin/env bash
# Host: log out of the Gnoblin guest session several times and check each shutdown is clean.
#
# Each cycle waits for the dwell, runs `gnoblinctl logout` and requires it to be accepted. It then checks the
# shutdown 10 s later:
# - the journal has no "Session never registered" from GDM
# Then it restarts GDM so autologin starts a new Gnoblin session, and checks:
# - no new core dump appeared (coredumpctl count)
# - the journal has no glibc corruption, SIGABRT or GLib critical from the old compositor
# - a new compositor process is running
# Set GNOBLIN_LOGOUT_CYCLES to change the number of cycles (default 8).
# Saves the output in the run directory.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
cycles="${GNOBLIN_LOGOUT_CYCLES:-8}"
# Seconds the new session runs before logout. GDM's session wrapper registers the session 10 s after it starts, so a
# shorter dwell is logged as "Session never registered".
dwell="${GNOBLIN_LOGOUT_DWELL:-45}"
out="$run/logout-cycles-$(date -u +%Y%m%dT%H%M%SZ).txt"
guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$1"; }

{
    echo "command: tests/qemu-e2e/run-logout-cycles.sh (cycles=$cycles dwell=$dwell)"
    echo "renderer: $(cat "$run/renderer" 2>/dev/null || echo unknown)"
    failures=0
    for n in $(seq 1 "$cycles"); do
        before="$(guest "$here/guest-logout-state.sh" | grep '^cores=')"
        sleep "$dwell"
        logout="$(guest "$here/guest-logout-trigger.sh" 2>&1 || true)"
        printf '%s\n' "$logout"
        case "$logout" in
            *'"accepted":true'*) accepted=1 ;;
            *) accepted=0 ;;
        esac
        sleep 10
        guest "$here/guest-logout-state.sh" > "$run/logout-cycle-$n-shutdown.txt" 2>&1 || true
        guest "$here/guest-logout-relogin.sh" || true
        ok=0
        for _ in $(seq 1 40); do
            sleep 3
            if guest "$here/guest-logout-state.sh" 2>/dev/null | grep -q '^compositor=running'; then
                ok=1
                break
            fi
        done
        state="$(guest "$here/guest-logout-state.sh" 2>&1 || true)"
        after="$(printf '%s\n' "$state" | grep '^cores=')"
        bad="$(printf '%s\n' "$state" | grep '^journal-bad=' || true)"
        # Read the registration check from the shutdown snapshot. The restart below can stop a greeter that has not
        # registered yet, and GDM logs that too.
        reg="$(grep '^registration-missing=' "$run/logout-cycle-$n-shutdown.txt" || true)"
        if [ "$accepted" -eq 1 ] && [ "$ok" -eq 1 ] && [ "$before" = "$after" ] && [ "$bad" = "journal-bad=0" ] && [ "$reg" = "registration-missing=0" ]; then
            echo "PASS cycle $n ($after, $bad, $reg)"
        else
            echo "FAIL cycle $n (accepted=$accepted ok=$ok before=[$before] after=[$after] $bad $reg)"
            printf '%s\n' "$state" | sed 's/^/    /'
            failures=$((failures + 1))
        fi
    done
    echo "RESULT failures=$failures"
} 2>&1 | tee "$out"
echo "saved: $out"
