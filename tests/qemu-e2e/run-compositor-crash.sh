#!/usr/bin/env bash
# Host: the session recovers after a compositor crash and says so once. Saves the output in the run directory.
#
# Crashes the compositor with SIGSEGV. The login that follows must start a working compositor, show the crash recovery
# marker with the signal, and keep the previous compositor log. A normal login after that must not show the marker again.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/compositor-crash-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0
guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s -- "$@" <"$here/guest-compositor-crash.sh"; }
field() { sed -n "s/^$1=//p"; }
check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}
{
    echo "command: tests/qemu-e2e/run-compositor-crash.sh"
    crash="$(guest crash)"
    check "the compositor is found before the crash" "$(field compositor_found <<<"$crash")" "yes"
    check "no Gnoblin process is left after SIGSEGV" "$(field processes_after_crash <<<"$crash")" "0"

    scripts/qemu-e2e guest "$run" -- sudo systemctl restart gdm
    sleep 30
    after="$(guest read)"
    check "the next login starts a compositor" "$(field status <<<"$after")" '"state":"running"'
    check "the recovery marker says crash" "$(field marker <<<"$after")" "crash"
    case "$(field marker_message <<<"$after")" in
        *"stopped by signal 11 (Segmentation fault)"*) echo "PASS the marker names signal 11" ;;
        *)
            echo "FAIL the marker should name signal 11 (got: $(field marker_message <<<"$after"))"
            fail=$((fail + 1))
            ;;
    esac
    case "$(field previous_log_bytes <<<"$after")" in
        missing | 0)
            echo "FAIL the previous compositor log is missing or empty"
            fail=$((fail + 1))
            ;;
        *) echo "PASS the previous compositor log is kept" ;;
    esac

    scripts/qemu-e2e guest "$run" -- sudo systemctl restart gdm
    sleep 30
    again="$(guest read)"
    check "a normal login after the crash starts a compositor" "$(field status <<<"$again")" '"state":"running"'
    check "a normal login after the crash shows no crash marker" "$(field marker <<<"$again")" ""
    echo "failures: $fail"
    exit "$fail"
} 2>&1 | tee "$out"
echo "saved: $out"
