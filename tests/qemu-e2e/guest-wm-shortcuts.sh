#!/usr/bin/env bash
# Guest: the standard GNOME window and workspace shortcuts work in a Gnoblin session.
#
# Mutter reads these from org.gnome.desktop.wm.keybindings, so they work without any Gnoblin config. The test opens a GTK
# window, gives it focus, presses each shortcut through RemoteDesktop, and checks the window state that gnoblinctl reports:
# Super+Up maximises, Super+Down restores, Alt+F10 toggles maximise, Super+H minimises, Ctrl+Alt+Right switches to the next
# workspace and Ctrl+Alt+Left back (workspaces are laid out in a row, as in GNOME 40 and later), and Alt+F4 closes a
# focused window. Each key is read from gsettings, so the test follows the
# defaults the session really has.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-wm-shortcuts.lua"
fail=0
trap 'pkill -f wm-shortcuts-app.py 2>/dev/null; rm -f "$F" /tmp/wm-shortcuts-app.py /tmp/wm-press.py; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$F"
"$G" config reload >/dev/null 2>&1
sleep 3

cat >/tmp/wm-shortcuts-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title=sys.argv[1])
window.set_default_size(320, 240)
window.set_child(Gtk.Label(label="shortcuts"))
window.present()
GLib.timeout_add_seconds(120, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

cat >/tmp/wm-press.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

CODES = {"ctrl": 29, "shift": 42, "alt": 56, "super": 125, "f4": 62, "f10": 68, "h": 35, "up": 103, "down": 108, "left": 105, "right": 106}
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
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], True)))
    time.sleep(0.1)
time.sleep(0.2)
for name in reversed(names):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], False)))
    time.sleep(0.1)
time.sleep(0.5)
call(session, iface, "Stop", None)
PY

window_field() {
    # window_field FIELD [TITLE]: prints a field of the window, or "none" when it is gone
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"${2:-wm-shortcuts}"'":
        print(w["'"$1"'"])
        break
else:
    print("none")'
}
workspace() { "$G" workspace list | python3 -c 'import json,sys; print(next(w["number"] for w in json.load(sys.stdin)["workspaces"] if w["active"]))'; }
press() {
    python3 /tmp/wm-press.py "$1" >/dev/null 2>&1
    sleep 1.5
}
binding() { gsettings get org.gnome.desktop.wm.keybindings "$1"; }

nohup env GDK_BACKEND=wayland python3 /tmp/wm-shortcuts-app.py wm-shortcuts >/dev/null 2>&1 </dev/null &
sleep 4

check "Super+Up is bound to maximize" "$(binding maximize)" "['<Super>Up']"
press super+up
check "Super+Up maximises the focused window" "$(window_field maximized)" "True"
press super+down
check "Super+Down restores the window" "$(window_field maximized)" "False"
press alt+f10
check "Alt+F10 maximises the window" "$(window_field maximized)" "True"
press alt+f10
check "Alt+F10 restores the window" "$(window_field maximized)" "False"

check "Super+H is bound to minimize" "$(binding minimize)" "['<Super>h']"
press super+h
check "Super+H minimises the window" "$(window_field minimized)" "True"
"$G" window unminimize "$(window_field id)" >/dev/null 2>&1
sleep 1

start="$(workspace)"
check "Ctrl+Alt+Right is bound to the next workspace" "$(binding switch-to-workspace-right | grep -c '<Control><Alt>Right')" "1"
press ctrl+alt+right
check "Ctrl+Alt+Right switches to the next workspace" "$(workspace)" "$((start + 1))"
press ctrl+alt+left
check "Ctrl+Alt+Left switches back" "$(workspace)" "$start"

# A new window takes focus under focus_new_windows = "allow", so Alt+F4 acts on it.
nohup env GDK_BACKEND=wayland python3 /tmp/wm-shortcuts-app.py wm-close >/dev/null 2>&1 </dev/null &
sleep 4
check "a new window is open to close" "$([ "$(window_field id wm-close)" != none ] && echo yes)" "yes"
press alt+f4
check "Alt+F4 closes the focused window" "$(window_field id wm-close)" "none"
check "Alt+F4 leaves the other window open" "$([ "$(window_field id)" != none ] && echo yes)" "yes"

echo "failures: $fail"
exit "$fail"
