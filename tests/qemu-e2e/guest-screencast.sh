#!/usr/bin/env bash
# Guest: a screen cast of a monitor delivers real frames over PipeWire.
#
# Screen sharing and recording tools get their frames from org.gnome.Mutter.ScreenCast. The test opens a session on the
# first monitor, takes the PipeWire node of its stream, and grabs one frame with GStreamer's pipewiresrc. It checks that the
# frame has the size of the monitor, that it shows a window the test opened and not only the black desktop, and that the
# compositor is still running after the session stops. Window casts and the portal dialog are not covered.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-screencast.lua"
STOP=/tmp/cast-stop
fail=0
trap 'touch "$STOP"; sleep 1; pkill -f cast-session.py 2>/dev/null; pkill -f cast-window.py 2>/dev/null; rm -f "$F" "$STOP" /tmp/cast-session.py /tmp/cast-window.py /tmp/cast-node.txt /tmp/cast-frame.png; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1
rm -f "$STOP" /tmp/cast-node.txt /tmp/cast-frame.png

printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$F"
"$G" config reload >/dev/null 2>&1
sleep 3

cat >/tmp/cast-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title="cast-window")
window.set_default_size(500, 400)
window.set_child(Gtk.Label(label="screen cast test"))
window.present()
GLib.timeout_add_seconds(90, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

# Holds a ScreenCast session open until /tmp/cast-stop exists, and writes the PipeWire node id of its stream.
cat >/tmp/cast-session.py <<'PY'
import os
import sys

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.ScreenCast", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/ScreenCast", "org.gnome.Mutter.ScreenCast", "CreateSession",
               GLib.Variant("(a{sv})", ({},))).unpack()[0]
stream = call(session, "org.gnome.Mutter.ScreenCast.Session", "RecordMonitor",
              GLib.Variant("(sa{sv})", (sys.argv[1], {}))).unpack()[0]
loop = GLib.MainLoop()


def on_added(connection, sender, path, interface, signal, params, data):
    open("/tmp/cast-node.txt", "w").write(str(params.unpack()[0]))


bus.signal_subscribe("org.gnome.Mutter.ScreenCast", "org.gnome.Mutter.ScreenCast.Stream", "PipeWireStreamAdded",
                     stream, None, Gio.DBusSignalFlags.NONE, on_added, None)
call(session, "org.gnome.Mutter.ScreenCast.Session", "Start", None)


def check_stop():
    if os.path.exists("/tmp/cast-stop"):
        call(session, "org.gnome.Mutter.ScreenCast.Session", "Stop", None)
        loop.quit()
        return False
    return True


GLib.timeout_add(250, check_stop)
GLib.timeout_add_seconds(60, lambda: (loop.quit(), False)[1])
loop.run()
PY

monitor="$("$G" monitor list --json | python3 -c 'import json,sys; m=json.load(sys.stdin)["monitors"][0]; print(m["id"], int(m["width"]*m["scale"]), int(m["height"]*m["scale"]))')"
read -r connector width height <<<"$monitor"

nohup env GDK_BACKEND=wayland python3 /tmp/cast-window.py >/dev/null 2>&1 </dev/null &
sleep 3
nohup python3 /tmp/cast-session.py "$connector" >/tmp/cast-session.log 2>&1 </dev/null &
for _ in $(seq 1 30); do
    if [ -s /tmp/cast-node.txt ]; then break; fi
    sleep 0.5
done
node="$(cat /tmp/cast-node.txt 2>/dev/null)"
check "the screen cast session gives a PipeWire node for $connector" "$([ -n "$node" ] && echo yes)" "yes"

if [ -n "$node" ]; then
    timeout 30 gst-launch-1.0 -q pipewiresrc path="$node" num-buffers=15 ! videoconvert ! pngenc snapshot=true ! \
        filesink location=/tmp/cast-frame.png >/tmp/cast-gst.log 2>&1
    check "GStreamer reads a frame from the stream" "$([ -s /tmp/cast-frame.png ] && echo yes)" "yes"
    result="$(
        python3 - <<'PY'
from PIL import Image

image = Image.open("/tmp/cast-frame.png").convert("RGB")
pixels = image.getdata()
lit = sum(1 for p in pixels if max(p) > 24)
print("%dx%d lit=%.3f" % (image.width, image.height, lit / len(pixels)))
PY
    )"
    check "the frame has the size of $connector" "${result%% *}" "${width}x${height}"
    lit="${result##*lit=}"
    check "the frame shows a window and not only the black desktop" "$(python3 -c "print('yes' if float('${lit:-0}') > 0.02 else 'no')")" "yes"
fi

touch "$STOP"
sleep 2
check "the compositor still runs after the cast stops" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
