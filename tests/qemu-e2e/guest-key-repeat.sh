#!/usr/bin/env bash
# Guest: the keyboard repeat settings from GNOME Settings change how a held key types.
#
# Mutter reads org.gnome.desktop.peripherals.keyboard. The test holds the A key for 1.5 seconds in a GTK4 text entry, with
# four settings, and counts the characters typed. A key typed after delay D with interval I gives about 1 + (1500 - D) / I
# characters: 35 for the defaults (500 ms and 30 ms), 8 with a 150 ms interval, 11 with a 1200 ms delay, and 1 with repeat
# off. The test accepts a range around each value, because timing in a virtual machine varies. It counts characters, not
# bytes. The settings are restored afterwards.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
K=org.gnome.desktop.peripherals.keyboard
F="$HOME/.config/gnoblin/config/99-test-key-repeat.lua"
fail=0
orig_repeat="$(gsettings get $K repeat)"
orig_delay="$(gsettings get $K delay | sed 's/uint32 //')"
orig_interval="$(gsettings get $K repeat-interval | sed 's/uint32 //')"
trap 'pkill -f key-repeat-app.py 2>/dev/null; gsettings set $K repeat "$orig_repeat"; gsettings set $K delay "$orig_delay"; gsettings set $K repeat-interval "$orig_interval"; rm -f "$F" /tmp/key-repeat-app.py /tmp/key-repeat-hold.py /tmp/key-repeat-entry.txt; "$G" config reload >/dev/null 2>&1' EXIT

in_range() {
    # in_range NAME ACTUAL LOW HIGH
    if [ "$2" -ge "$3" ] && [ "$2" -le "$4" ]; then
        echo "PASS $1 ($2 characters, expected $3 to $4)"
    else
        echo "FAIL $1 ($2 characters, expected $3 to $4)"
        fail=$((fail + 1))
    fi
}

printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$F"
"$G" config reload >/dev/null 2>&1
sleep 3

cat >/tmp/key-repeat-app.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="key-repeat-test")
window.set_default_size(500, 100)
entry = Gtk.Entry()
window.set_child(entry)
window.present()
entry.grab_focus()
entry.connect("changed", lambda w: open("/tmp/key-repeat-entry.txt", "w").write(w.get_text()))
loop = GLib.MainLoop()
GLib.timeout_add_seconds(60, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

cat >/tmp/key-repeat-hold.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (30, True)))
time.sleep(float(sys.argv[1]))
call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (30, False)))
time.sleep(0.3)
call(session, iface, "Stop", None)
PY

held() {
    # held SECONDS: a fresh entry, the A key held, the number of characters typed
    pkill -f key-repeat-app.py
    sleep 0.5
    : >/tmp/key-repeat-entry.txt
    nohup env GDK_BACKEND=wayland python3 /tmp/key-repeat-app.py >/dev/null 2>&1 </dev/null &
    sleep 3
    python3 /tmp/key-repeat-hold.py "$1"
    python3 -c 'print(len(open("/tmp/key-repeat-entry.txt").read()))'
}

gsettings set $K repeat true
gsettings set $K delay 500
gsettings set $K repeat-interval 30
sleep 1
in_range "the default settings (500 ms delay, 30 ms interval)" "$(held 1.5)" 28 40
gsettings set $K repeat-interval 150
sleep 1
in_range "a 150 ms interval" "$(held 1.5)" 6 10
gsettings set $K repeat-interval 30
gsettings set $K delay 1200
sleep 1
in_range "a 1200 ms delay" "$(held 1.5)" 8 14
gsettings set $K delay 500
gsettings set $K repeat false
sleep 1
in_range "repeat switched off" "$(held 1.5)" 1 1

echo "failures: $fail"
exit "$fail"
