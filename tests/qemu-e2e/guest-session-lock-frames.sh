#!/usr/bin/env bash
# Guest: a locker that repaints on wl_surface.frame callbacks (the Qt Wayland pattern) must receive them while it holds
# the lock. Without them the locker never paints its real UI and the session cannot be unlocked from its own screen.
# The locker (session-lock-frame-client) takes the lock, paints, and repaints on every callback. After five callbacks per
# output it unlocks, tears its surfaces down like Qt (a null-buffer commit after the role object is gone) and exits 0; with no callback inside 15 s it exits 1 and leaves the session in the failsafe state,
# which swaylock then clears.
# The host script puts the locker at /tmp/session-lock-frame-client before this part runs.
set -u
chmod +x /tmp/session-lock-frame-client
uid="$(id -u)"
export XDG_RUNTIME_DIR="/run/user/$uid"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
fail=0
trap 'pkill -9 -x swaylock 2>/dev/null; rm -f /tmp/session-lock-frame-client /tmp/lock-frames.log' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}
lock_state() { "$G" status 2>&1 | head -1 | grep -o '"lock_state":"[a-z-]*"'; }

check "the session starts unlocked" "$(lock_state)" '"lock_state":"unlocked"'
timeout 40 /tmp/session-lock-frame-client 5 15 >/tmp/lock-frames.log 2>&1
rc=$?
sed 's/^/  locker: /' /tmp/lock-frames.log
check "the locker gets five frame callbacks on every output and unlocks" "$rc" "0"
sleep 2
check "the session is unlocked after the locker exits" "$(lock_state)" '"lock_state":"unlocked"'

if [ "$(lock_state)" != '"lock_state":"unlocked"' ]; then
    # Recover the failsafe lock so later checks start clean.
    swaylock -f -c 336699 >/dev/null 2>&1
    sleep 3
    pkill -USR1 -x swaylock
    sleep 3
fi
echo "failures: $fail"
exit "$fail"
