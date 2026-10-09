#!/usr/bin/env bash
# Guest: Alt+Tab, Alt+Shift+Tab, Super+Tab and Alt+Esc move focus between windows.
#
# Mutter's built-in switch_applications and cycle_windows actions do this with no shell running. They have
# no popup, which a shell draws. The test opens two windows side by side, focuses one with a real click,
# presses each combination through RemoteDesktop, and requires focus to change to the other window.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
fail=0

cat > /tmp/press-keys.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

KEYS = {"alt": 56, "shift": 42, "tab": 15, "esc": 1, "super": 125}
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
names = sys.argv[1].split("+")
for name in names:
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEYS[name], True)))
    time.sleep(0.1)
time.sleep(0.2)
for name in reversed(names):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEYS[name], False)))
    time.sleep(0.1)
time.sleep(0.5)
call(session, iface, "Stop", None)
PY

focused() {
    "$G" window list | python3 -c '
import json, sys
print(",".join(w["title"] for w in json.load(sys.stdin)["windows"] if w.get("focused")) or "none")'
}
window_id() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        print(w["id"])'
}
center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}

nohup swaybg -c "#303030" >/dev/null 2>&1 < /dev/null &
nohup foot -T win-A >/dev/null 2>&1 < /dev/null &
sleep 3
nohup foot -T win-B >/dev/null 2>&1 < /dev/null &
sleep 3
# Put the windows side by side so a click reaches the one it aims at.
"$G" window move "$(window_id win-A)" 40 100 >/dev/null 2>&1
"$G" window move "$(window_id win-B)" 760 100 >/dev/null 2>&1
sleep 1

for combo in alt+tab alt+shift+tab super+tab alt+esc; do
    python3 "$CLICK" $(center win-A) >/dev/null 2>&1
    sleep 1
    python3 "$CLICK" $(center win-B) >/dev/null 2>&1
    sleep 1
    before="$(focused)"
    python3 /tmp/press-keys.py "$combo"
    sleep 1
    after="$(focused)"
    if [ "$before" = "win-B" ] && [ "$after" = "win-A" ]; then
        echo "PASS $combo moves focus $before -> $after"
    else
        echo "FAIL $combo moves focus $before -> $after (expected win-B -> win-A)"
        fail=$((fail + 1))
    fi
done

pkill -f "foot -T win-"
pkill -x swaybg
echo "failures: $fail"
exit "$fail"
