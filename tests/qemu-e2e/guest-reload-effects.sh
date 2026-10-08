#!/usr/bin/env bash
# Guest: two config sections change behaviour on reload without a new session.
#
# autostart: adding an entry starts its command, enable = false stops it, enabling it again starts it again,
#            and removing the config stops it.
# window_management.focus_mode: "click" keeps focus when the pointer moves onto another window, "sloppy"
#            focuses the window under the pointer.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
F="$HOME/.config/gnoblin/config/99-test-reload-effects.lua"
fail=0

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

apply() {
    printf '%s\n' "$1" > "$F"
    "$G" config reload >/dev/null
    sleep 4
}

marker_count() { pgrep -fc 'gnoblin-autostart-marker-sleep' 2>/dev/null || true; }

ENTRY='{command = {"sh", "-c", "exec -a gnoblin-autostart-marker-sleep sleep 600"}'
check "no marker process before the test" "$(marker_count)" "0"
apply "gnoblin.configure {autostart = {marker = $ENTRY}}}"
check "adding an autostart entry starts it" "$(marker_count)" "1"
apply "gnoblin.configure {autostart = {marker = ${ENTRY}, enable = false}}}"
check "enable = false stops it" "$(marker_count)" "0"
apply "gnoblin.configure {autostart = {marker = $ENTRY}}}"
check "enabling it again starts it again" "$(marker_count)" "1"
apply '-- disabled'
check "removing the config stops it" "$(marker_count)" "0"

cat > /tmp/pointer-move.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

x, y = float(sys.argv[1]), float(sys.argv[2])
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
time.sleep(0.2)
steps = 10
for _ in range(steps):
    call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (x / steps, y / steps)))
    time.sleep(0.05)
time.sleep(0.6)
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
nohup foot -T focus-A >/dev/null 2>&1 < /dev/null &
sleep 3
nohup foot -T focus-B >/dev/null 2>&1 < /dev/null &
sleep 3
"$G" window move "$(window_id focus-A)" 40 100 >/dev/null 2>&1
"$G" window move "$(window_id focus-B)" 760 100 >/dev/null 2>&1
sleep 1

for mode in click sloppy; do
    apply "gnoblin.configure {window_management = {focus_mode = \"$mode\"}}"
    python3 "$CLICK" $(center focus-A) >/dev/null 2>&1
    sleep 1
    before="$(focused)"
    python3 /tmp/pointer-move.py $(center focus-B)
    sleep 1
    after="$(focused)"
    if [ "$mode" = click ]; then
        check "focus_mode click keeps focus when the pointer enters another window" "$before -> $after" "focus-A -> focus-A"
    else
        check "focus_mode sloppy focuses the window under the pointer" "$before -> $after" "focus-A -> focus-B"
    fi
done

apply '-- disabled'
rm -f "$F"
pkill -f "foot -T focus-"
pkill -x swaybg
echo "failures: $fail"
exit "$fail"
