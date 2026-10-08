#!/usr/bin/env bash
# Guest: the session survives a crash of Xwayland, and X11 apps work again afterwards.
#
# Mutter makes Xwayland mandatory when the compositor does not run as a systemd user unit, and exits when it exits. Gnoblin
# uses the on-demand policy, as a GNOME Shell session does. The test checks that Xwayland is not running until an X11 app
# connects, kills it with SIGKILL while an X11 app and a Wayland app are open, and checks that the compositor and the
# Wayland window survive, that the X11 window is gone, and that a new X11 app gets a fresh Xwayland.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
fail=0
trap 'pkill -f xw-app.py 2>/dev/null; rm -f /tmp/xw-app.py' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat >/tmp/xw-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title=sys.argv[1])
window.set_default_size(300, 200)
window.set_child(Gtk.Label(label=sys.argv[1]))
window.present()
GLib.timeout_add_seconds(60, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

titles() { "$G" window list | python3 -c 'import json,sys; print(" ".join(sorted(w["title"] for w in json.load(sys.stdin)["windows"] if w["title"].startswith("xw-"))))'; }
compositor() { "$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"'; }
xwayland_count() { pgrep -x -c Xwayland; }
DISPLAY=:0
XAUTHORITY="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
export DISPLAY XAUTHORITY

# Xwayland may be left from an earlier test, so end it and let the compositor settle.
pkill -x Xwayland 2>/dev/null
sleep 3
check "Xwayland is not running until an X11 app connects" "$(xwayland_count)" "0"

nohup env GDK_BACKEND=wayland python3 /tmp/xw-app.py xw-wayland >/dev/null 2>&1 </dev/null &
nohup env GDK_BACKEND=x11 python3 /tmp/xw-app.py xw-x11-one >/dev/null 2>&1 </dev/null &
sleep 6
check "an X11 app starts Xwayland" "$(xwayland_count)" "1"
check "both windows are open" "$(titles)" "xw-wayland xw-x11-one"

pkill -9 -x Xwayland
sleep 3
check "the compositor still runs after Xwayland is killed" "$(compositor)" '"state":"running"'
check "the Wayland window survives and the X11 window is gone" "$(titles)" "xw-wayland"

nohup env GDK_BACKEND=x11 python3 /tmp/xw-app.py xw-x11-two >/dev/null 2>&1 </dev/null &
sleep 6
check "a new X11 app starts a fresh Xwayland" "$(xwayland_count)" "1"
check "the new X11 window and the Wayland window are open" "$(titles)" "xw-wayland xw-x11-two"

echo "failures: $fail"
exit "$fail"
