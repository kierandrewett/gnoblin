#!/usr/bin/env bash
# Guest: compositor.locate_pointer sends gnoblin.pointer.locate-requested and draws nothing.
#
# With locate_pointer = false the key press must send no event. With true it must send one event that
# carries the pointer position. A Lua listener (configs/99-test-locate.lua) logs each event.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
LISTENER="$HOME/.config/gnoblin/config/99-test-locate.lua"
SETTING="$HOME/.config/gnoblin/config/99-test-locate-setting.lua"
LOG=/tmp/locate-events.log
fail=0

cp /tmp/99-test-locate.lua "$LISTENER"

cat > /tmp/press-f12-locate.py <<'PY'
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
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
time.sleep(0.2)
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (420.0, 260.0)))
time.sleep(0.5)
call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (88, True)))
time.sleep(0.05)
call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (88, False)))
time.sleep(1.0)
call(session, iface, "Stop", None)
PY

press() {
    printf 'gnoblin.configure {compositor = {locate_pointer = %s, locate_pointer_key = "F12"}}\n' "$1" > "$SETTING"
    "$G" config reload >/dev/null
    sleep 3
    : > "$LOG"
    python3 /tmp/press-f12-locate.py
    sleep 2
    grep -c '^locate ' "$LOG"
}

off="$(press false)"
on="$(press true)"
event="$(grep '^locate ' "$LOG" | head -1)"

if [ "$off" -eq 0 ]; then
    echo "PASS locate_pointer=false sends no event"
else
    echo "FAIL locate_pointer=false sent $off event(s)"
    fail=$((fail + 1))
fi
if [ "$on" -ge 1 ]; then
    echo "PASS locate_pointer=true sends an event ($event)"
else
    echo "FAIL locate_pointer=true sent no event"
    fail=$((fail + 1))
fi
case "$event" in
    *"x=4"*"y=2"*)
        echo "PASS the event carries the pointer position" ;;
    *)
        echo "FAIL the event has no usable pointer position ($event)"
        fail=$((fail + 1)) ;;
esac

rm -f "$LISTENER" "$SETTING"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
