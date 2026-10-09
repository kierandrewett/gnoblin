#!/usr/bin/env bash
# Guest: lock requests from D-Bus and logind reach a shell that subscribed to them.
#
# Gnoblin has no lock screen. A shell subscribes to gnoblin.session.lock-requested and shows its own. This test plays
# that shell with a Lua script in gnoblinctl, which starts swaylock when a request arrives. It then checks the triggers:
#   - org.freedesktop.ScreenSaver.Lock with no shell subscribed fails and says why;
#   - ScreenSaver.Lock with a shell subscribed reaches it, and the shell's locker takes the lock;
#   - GetActive reports the lock, a second Lock request succeeds without a second event, and SetActive(false) is refused;
#   - the key binding documented in docs/session-lock.md, pressed through RemoteDesktop, reaches the shell;
#   - loginctl lock-session for the graphical session reaches the shell, and for another session does not.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
REQUESTS=/tmp/lock-requests.txt
SHELL_LUA=/tmp/lock-shell.lua
fail=0
BINDING_CONFIG="$HOME/.config/gnoblin/config/99-test-lock-request.lua"
trap 'pkill -9 -x swaylock 2>/dev/null; pkill -f "lock-shell.lua" 2>/dev/null; rm -f "$SHELL_LUA" "$REQUESTS" "$BINDING_CONFIG" /tmp/press-keys.py; "$G" config reload >/dev/null 2>&1' EXIT

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
wait_for_state() {
    # wait_for_state STATE: wait up to 10 seconds
    for _ in $(seq 1 40); do
        if [ "$(lock_state)" = "\"lock_state\":\"$1\"" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}
request_count() { wc -l <"$REQUESTS" 2>/dev/null | tr -d ' '; }
wait_for_requests() {
    # wait_for_requests COUNT: wait up to 5 seconds for the shell to have seen COUNT requests
    for _ in $(seq 1 20); do
        if [ "$(request_count)" -ge "$1" ] 2>/dev/null; then return 0; fi
        sleep 0.25
    done
    return 1
}
screensaver() {
    busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver "$@" 2>&1
}

: >"$REQUESTS"
check "the session starts unlocked" "$(lock_state)" '"lock_state":"unlocked"'

out="$(screensaver Lock)"
case "$out" in
    *"no session-lock client is subscribed"*) echo "PASS Lock with no shell subscribed fails and says why" ;;
    *)
        echo "FAIL Lock with no shell subscribed should fail with the reason (got: $out)"
        fail=$((fail + 1))
        ;;
esac
check "GetActive says no with the session unlocked" "$(screensaver GetActive)" "b false"

cat >"$SHELL_LUA" <<'LUA'
gnoblin.events.on("gnoblin.session.lock-requested", function(event)
    local file = io.open("/tmp/lock-requests.txt", "a")
    file:write("requested\n")
    file:close()
end)
LUA
nohup "$G" lua "$SHELL_LUA" >/tmp/lock-shell.log 2>&1 </dev/null &
sleep 2

screensaver Lock >/dev/null
check "Lock reaches the subscribed shell" "$(wait_for_requests 1 && echo seen)" "seen"
swaylock -f -c 336699 >/dev/null 2>&1
check "the shell's locker takes the lock" "$(wait_for_state locked && echo locked)" "locked"
check "GetActive says yes while locked" "$(screensaver GetActive)" "b true"

before="$(request_count)"
out="$(screensaver Lock)"
check "a second Lock while locked succeeds" "${out:-ok}" "ok"
sleep 1
check "a second Lock while locked sends the shell no new request" "$(request_count)" "$before"
out="$(screensaver SetActive b false)"
case "$out" in
    *"Only the lock client can unlock"*) echo "PASS SetActive(false) is refused" ;;
    *)
        echo "FAIL SetActive(false) should be refused (got: $out)"
        fail=$((fail + 1))
        ;;
esac
check "the session is still locked after SetActive(false)" "$(lock_state)" '"lock_state":"locked"'
pkill -USR1 -x swaylock
check "the locker unlocks the session" "$(wait_for_state unlocked && echo unlocked)" "unlocked"

# gnoblin.session.lock() from a Lua script reaches the shell and reports one subscriber.
BIND_LUA=/tmp/lock-bind.lua
cat >"$BIND_LUA" <<'LUA'
local request = gnoblin.session.lock()
print(request.dispatched, request.subscribers)
LUA
before="$(request_count)"
"$G" lua "$BIND_LUA" >/tmp/lock-bind.log 2>&1
check "gnoblin.session.lock() in Lua reaches the shell" "$(wait_for_requests $((before + 1)) && echo seen)" "seen"
check "gnoblin.session.lock() reports one subscriber" "$(awk '{print $2}' /tmp/lock-bind.log)" "1"
swaylock -f -c 336699 >/dev/null 2>&1
wait_for_state locked || true
pkill -USR1 -x swaylock
wait_for_state unlocked || true
rm -f "$BIND_LUA" /tmp/lock-bind.log

# The key binding from docs/session-lock.md, written exactly as documented, then Super+L pressed through RemoteDesktop.
cat >"$BINDING_CONFIG" <<'LUA'
gnoblin.configure {
    keybindings = {
        keyboard = {
            lock_screen = {
                binding = "<Super>l",
                callback = function()
                    gnoblin.session.lock()
                end,
            },
        },
    },
}
LUA
"$G" config reload >/dev/null 2>&1
sleep 3
cat >/tmp/press-keys.py <<'PY'
import time

from gi.repository import Gio, GLib

KEYS = [125, 38]  # Super, L
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
for code in KEYS:
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, True)))
    time.sleep(0.1)
time.sleep(0.2)
for code in reversed(KEYS):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, False)))
    time.sleep(0.1)
time.sleep(0.5)
call(session, iface, "Stop", None)
PY
before="$(request_count)"
python3 /tmp/press-keys.py
check "the documented Super+L key binding reaches the shell" "$(wait_for_requests $((before + 1)) && echo seen)" "seen"
swaylock -f -c 336699 >/dev/null 2>&1
wait_for_state locked || true
pkill -USR1 -x swaylock
wait_for_state unlocked || true

display="$(loginctl show-user "$(id -u)" -p Display --value)"
other="$(loginctl list-sessions --no-legend | awk -v uid="$(id -un)" -v d="$display" '$3 == uid && $1 != d {print $1; exit}')"
before="$(request_count)"
loginctl lock-session "$display"
check "loginctl lock-session for the graphical session reaches the shell" "$(wait_for_requests $((before + 1)) && echo seen)" "seen"
swaylock -f -c 336699 >/dev/null 2>&1
check "the shell's locker takes the lock after loginctl" "$(wait_for_state locked && echo locked)" "locked"
pkill -USR1 -x swaylock
wait_for_state unlocked || true

# logind refuses to lock a session that has no lock screen, such as an SSH login, and sends no signal. Without a signal
# there is nothing to test, so the check only counts when logind accepts the request.
if [ -n "$other" ]; then
    before="$(request_count)"
    if loginctl lock-session "$other" 2>/dev/null; then
        sleep 2
        check "loginctl lock-session for another session sends the shell no request" "$(request_count)" "$before"
    else
        echo "SKIP loginctl lock-session for another session: logind refused it, so no signal was sent"
    fi
else
    echo "SKIP loginctl lock-session for another session: this user has no second session"
fi

echo "failures: $fail"
exit "$fail"
