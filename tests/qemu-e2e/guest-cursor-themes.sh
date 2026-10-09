#!/usr/bin/env bash
# Guest: capture the compositor cursor for several cursor.theme and cursor.size values.
#
# This works without hyprcursor installed. It builds a one-shape Xcursor theme in the user's home, then
# captures the screen with the cursor at 128 px and 192 px. Sprites of 64 px or less go to the hardware
# cursor plane, which screen captures leave out, so the test uses sizes the compositor draws itself.
# The host script run-cursor-themes.sh copies /tmp/cur-*.png back and compares them.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-cursor.lua"
T="$HOME/.local/share/icons/GnoblinTest"

mkdir -p "$T/cursors"
printf '[Icon Theme]\nName=GnoblinTest\n' > "$T/index.theme"
for shape in left_ptr default arrow; do
    cp /usr/share/icons/Adwaita/cursors/crosshair "$T/cursors/$shape"
done
rm -f /tmp/cur-*.png

python3 - <<'PY'
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
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (300.0, 200.0)))
time.sleep(0.5)
call(session, iface, "Stop", None)
PY

nohup swaybg -c "#808080" >/dev/null 2>&1 < /dev/null &
sleep 2

shot() {
    printf 'gnoblin.configure {cursor = {theme = "%s", size = %s}}\n' "$2" "$3" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    grim -c "/tmp/cur-$1.png"
}
shot default default 128
shot custom GnoblinTest 128
shot missing no-such-theme-xyz 128
shot large default 192

for bad in 0 257 -5; do
    printf 'gnoblin.configure {cursor = {theme = "default", size = %s}}\n' "$bad" > "$F"
    if "$G" config reload >/dev/null 2>&1; then
        echo "reject-size-$bad=accepted"
    else
        echo "reject-size-$bad=rejected"
    fi
done
printf -- '-- disabled\n' > "$F"
"$G" config reload >/dev/null
pkill -x swaybg
echo "compositor-alive=$(pgrep -fc 'bin/gnoblin --wayland')"
