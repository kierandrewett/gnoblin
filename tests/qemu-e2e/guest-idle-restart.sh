#!/usr/bin/env bash
# Guest: the idle and screen saver service comes back after it is killed.
#
# gnoblin-idle owns org.freedesktop.ScreenSaver. Media players use it to keep the screen awake, and lock requests go through
# it. The test kills the service with SIGKILL and checks that systemd starts it again, that it owns its D-Bus name again, and
# that calls to it work. The caller of a lost Inhibit must ask again; that is not tested.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
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

owner() { busctl --user list --no-pager | awk '$1=="org.freedesktop.ScreenSaver" {print $2}'; }
unit_state() { systemctl --user is-active gnoblin-idle.service; }
wait_for_owner() {
    for _ in $(seq 1 40); do
        if [ -n "$(owner)" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}

check "the idle service runs before the kill" "$(unit_state)" "active"
before="$(owner)"
check "the idle service owns org.freedesktop.ScreenSaver" "$([ -n "$before" ] && echo yes)" "yes"

pkill -9 -f libexec/gnoblin-idle
sleep 0.5
wait_for_owner
check "systemd starts the idle service again" "$(unit_state)" "active"
after="$(owner)"
check "the new service owns org.freedesktop.ScreenSaver" "$([ -n "$after" ] && echo yes)" "yes"
check "the new service is a new process" "$([ "$after" != "$before" ] && echo yes)" "yes"
check "a call to the new service works" \
    "$(busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver GetActive 2>&1)" \
    "b false"
check "the new service accepts an Inhibit" \
    "$(busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver Inhibit ss test restart 2>&1 | cut -d' ' -f1)" \
    "u"

echo "failures: $fail"
exit "$fail"
