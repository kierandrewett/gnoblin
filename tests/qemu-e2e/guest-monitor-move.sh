#!/usr/bin/env bash
# Guest: Super+Shift+Right and Super+Shift+Left move a window to the next and previous monitor, as in GNOME, and the
# window then maximizes to the area of the monitor it is on.
#
# The guest has two virtual monitors of the same size side by side, so the expected positions come from the width of the
# first monitor. A window on the first monitor is moved right, maximized with Super+Up, and moved back left.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
fail=0
trap 'pkill -f "foot -T mon-test" 2>/dev/null' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat > /tmp/press-keys.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

KEYS = {"shift": 42, "super": 125, "up": 103, "down": 108, "left": 105, "right": 106}
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

# Prints "MONITOR X Y W H MAXIMIZED" for the test window.
state() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "mon-test":
        f = w["frame"]
        print("%s %d %d %d %d %s" % (w["monitor_id"], f["x"], f["y"], f["width"], f["height"], w["maximized"]))'
}
center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "mon-test":
        f = w["frame"]; print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}
read -r mon_w mon_h <<<"$("$G" monitor list | python3 -c '
import json, sys
data = json.load(sys.stdin)
monitors = data.get("monitors", data) if isinstance(data, dict) else data
m = monitors[0]
box = m.get("rect") or m.get("geometry") or m
print(m.get("width", box.get("width")), m.get("height", box.get("height")))')"

nohup foot -T mon-test >/dev/null 2>&1 < /dev/null &
sleep 4
python3 "$CLICK" $(center) >/dev/null 2>&1
sleep 1

read -r m0 x0 y0 w0 h0 max0 <<<"$(state)"
check "the window starts on the first monitor" "$m0" "Virtual-1"

python3 /tmp/press-keys.py super+shift+right; sleep 1.5
read -r m1 x1 y1 w1 h1 max1 <<<"$(state)"
check "Super+Shift+Right moves the window to the second monitor" "$m1" "Virtual-2"
check "the move keeps the size and the position on the monitor" "$x1 $y1 $w1 $h1" "$((x0 + mon_w)) $y0 $w0 $h0"

python3 /tmp/press-keys.py super+up; sleep 1.5
read -r m2 x2 y2 w2 h2 max2 <<<"$(state)"
check "Super+Up on the second monitor maximizes to that monitor's area" "$m2 $x2 $y2 $w2 $h2 $max2" "Virtual-2 $mon_w 0 $mon_w $mon_h True"

python3 /tmp/press-keys.py super+shift+left; sleep 1.5
read -r m3 x3 y3 w3 h3 max3 <<<"$(state)"
check "Super+Shift+Left moves the maximized window back and it fills the first monitor" "$m3 $x3 $y3 $w3 $h3 $max3" "Virtual-1 0 0 $mon_w $mon_h True"

echo "failures: $fail"
exit "$fail"
