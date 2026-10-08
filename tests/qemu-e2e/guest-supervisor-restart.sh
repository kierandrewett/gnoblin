#!/usr/bin/env bash
# Guest: the session recovers when the session supervisor is killed.
#
# The guardian starts the compositor and the session supervisor, and the supervisor starts the Lua worker. The test kills the
# supervisor with SIGKILL and checks that the guardian starts a new one, that a new worker follows, that the compositor keeps
# running, and that a config reload still works, which needs the new worker.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
fail=0

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

process_with() {
    # process_with FLAG: the pid of the gnoblin process started with FLAG
    local pid
    for pid in $(pgrep -x gnoblin); do
        if tr '\0' ' ' <"/proc/$pid/cmdline" | grep -q -e "$1"; then
            echo "$pid"
            return
        fi
    done
}
wait_for_new() {
    # wait_for_new FLAG OLD_PID: wait up to 15 seconds for a process with FLAG other than OLD_PID
    for _ in $(seq 1 60); do
        new="$(process_with "$1")"
        if [ -n "$new" ] && [ "$new" != "$2" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}

supervisor="$(process_with '--internal-session-supervisor')"
worker="$(process_with '--internal-runtime-worker')"
compositor="$(process_with '--wayland')"
check "the supervisor, the worker and the compositor run before the kill" "$([ -n "$supervisor" ] && [ -n "$worker" ] && [ -n "$compositor" ] && echo yes)" "yes"

kill -9 "$supervisor"
check "the guardian starts a new supervisor" "$(wait_for_new '--internal-session-supervisor' "$supervisor" && echo yes)" "yes"
check "a new worker follows" "$(wait_for_new '--internal-runtime-worker' "$worker" && echo yes)" "yes"
sleep 2
check "the compositor is the same process and still runs" "$(process_with '--wayland')" "$compositor"
check "the compositor reports running" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'
check "a config reload works with the new worker" "$("$G" config reload >/dev/null 2>&1 && echo ok)" "ok"

echo "failures: $fail"
exit "$fail"
