#!/usr/bin/env bash
# Guest: ring the system bell with compositor.visual_bell off and on, capturing a burst of frames each time.
# The host script run-bell.sh copies /tmp/bell-*.png back and compares the frames.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-bell.lua"

cat > /tmp/beep.py <<'PY'
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk

Gtk.init()
display = Gdk.Display.get_default()
loop = GLib.MainLoop()
rings = 0


def ring():
    # The flash lasts about 100 ms, so ring often enough that the frame burst overlaps one.
    global rings
    display.beep()
    rings += 1
    if rings >= 10:
        loop.quit()
        return False
    return True


GLib.timeout_add(250, ring)
loop.run()
PY

cat > /tmp/beep-window.py <<'PY'
import os

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

rings = 0


def on_activate(app):
    window = Gtk.ApplicationWindow(application=app, title="bell-test")
    window.set_default_size(500, 400)
    window.present()

    def ring():
        global rings
        if not os.path.exists("/tmp/bell-go"):
            return True
        window.get_surface().beep()
        rings += 1
        if rings >= 10:
            app.quit()
            return False
        return True

    GLib.timeout_add(250, ring)


app = Gtk.Application(application_id="dev.gnoblin.BellTest")
app.connect("activate", on_activate)
app.run([])
PY

burst() {
    rm -f /tmp/bell-"$1"-*.png
    python3 /tmp/beep.py &
    beep_pid=$!
    for n in $(seq 1 60); do
        grim "/tmp/bell-$1-$(printf %03d "$n").png"
    done
    wait "$beep_pid"
}

apply() {
    printf '%s\n' "$1" > "$F"
    "$G" config reload >/dev/null
    sleep 3
}

rm -f /tmp/bell-*.png
nohup swaybg -c "#202020" >/dev/null 2>&1 < /dev/null &
sleep 2
apply 'gnoblin.configure {compositor = {visual_bell = false, audible_bell = false}}'
burst off
apply 'gnoblin.configure {compositor = {visual_bell = true, audible_bell = false, visual_bell_type = "fullscreen-flash"}}'
burst on
apply 'gnoblin.configure {compositor = {visual_bell = true, audible_bell = false, visual_bell_type = "frame-flash"}}'
# frame-flash flashes the window that rang the bell, so ring from a focused window's own surface.
rm -f /tmp/bell-go /tmp/bell-frame-*.png
nohup python3 /tmp/beep-window.py >/dev/null 2>&1 < /dev/null &
sleep 4
center="$("$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "bell-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)')"
python3 "${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}" $center >/dev/null 2>&1
sleep 2
touch /tmp/bell-go
for n in $(seq 1 60); do
    grim "/tmp/bell-frame-$(printf %03d "$n").png"
done
rm -f /tmp/bell-go
pkill -f /tmp/beep-window.py
apply '-- disabled'
pkill -x swaybg
echo "captured: $(ls /tmp/bell-*.png | wc -l) frames"
