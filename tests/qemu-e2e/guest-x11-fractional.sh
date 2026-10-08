#!/usr/bin/env bash
# Guest: a GTK X11 app keeps its logical size when the monitors use a fractional scale.
#
# Sets both virtual monitors to a fractional scale with gdctl, opens a 400x300 GTK4 window on the X11 backend, and
# compares the compositor's frame size with the frame size at scale 1.0. XSETTINGS must give the app scale factor 2
# (Mutter rounds the X11 scale up). The layout is restored to scale 1.0 at the end.
#
# GNOBLIN_TEST_SCALE selects the scale (default 1.25). 1280x800 gives whole-number logical sizes at 1.25 but not at
# 1.5, so GNOBLIN_TEST_MODE can select another monitor mode, for example 1920x1080@60.000 for 1.5. The baseline at
# scale 1.0 uses the same mode, and the restore returns the monitors to their preferred mode.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
GD="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gdctl"
G="${GNOBLIN_PREFIX}/bin/gnoblinctl"
export XAUTHORITY="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)" DISPLAY=:0
SCALE="${GNOBLIN_TEST_SCALE:-1.25}"
MODE="${GNOBLIN_TEST_MODE:-}"
fail=0

cat > /tmp/x11-size.py <<'PY'
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="x11-size-test")
window.set_default_size(400, 300)
window.set_child(Gtk.Label(label="size test"))
window.present()
loop = GLib.MainLoop()


def report():
    surface = window.get_surface()
    open("/tmp/x11-scale.txt", "w").write("scale_factor=%s\n" % (surface.get_scale_factor() if surface else None))
    return False


GLib.timeout_add(2500, report)
GLib.timeout_add_seconds(9, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

set_scale() {
    # set_scale SCALE [MODE]. Without a mode the monitors return to their preferred one.
    local mode_args=()
    if [ -n "${2:-}" ]; then mode_args=(--mode "$2"); fi
    "$GD" set --logical-monitor --primary --monitor Virtual-1 "${mode_args[@]}" --scale "$1" \
        --logical-monitor --monitor Virtual-2 "${mode_args[@]}" --scale "$1" --right-of Virtual-1 >/dev/null 2>&1
}

measure() {
    # Prints "WIDTHxHEIGHT SCALE_FACTOR" for the test window.
    sleep 3
    rm -f /tmp/x11-scale.txt
    GDK_BACKEND=x11 timeout 12 python3 /tmp/x11-size.py >/dev/null 2>&1 &
    sleep 4.5
    frame="$("$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "x11-size-test":
        f = w["frame"]
        print("%dx%d" % (f["width"], f["height"]))')"
    wait
    echo "$frame $(cat /tmp/x11-scale.txt 2>/dev/null)"
}

if [ -n "$MODE" ]; then set_scale 1 "$MODE"; fi
base="$(measure)"
set_scale "$SCALE" "$MODE"
fractional="$(measure)"
set_scale 1
sleep 3
restored="$("$GD" show 2>&1 | grep -m1 -o 'Scale: [0-9.]*')"

echo "scale 1.0:  $base"
echo "scale $SCALE: $fractional"
if [ "${base%% *}" = "${fractional%% *}" ] && [ -n "${base%% *}" ]; then
    echo "PASS the X11 window keeps its logical size (${base%% *}) at scale $SCALE"
else
    echo "FAIL the X11 window changed size at scale 1.25 (1.0: ${base%% *}, $SCALE: ${fractional%% *})"
    fail=$((fail + 1))
fi
case "$fractional" in
    *"scale_factor=2"*) echo "PASS the X11 app sees scale factor 2 at monitor scale $SCALE" ;;
    *) echo "FAIL the X11 app scale factor is wrong at monitor scale $SCALE ($fractional)"; fail=$((fail + 1)) ;;
esac
case "$restored" in
    *"Scale: 1.0"*) echo "PASS the monitor layout is back at scale 1.0" ;;
    *) echo "FAIL the monitor layout was not restored ($restored)"; fail=$((fail + 1)) ;;
esac
echo "failures: $fail"
exit "$fail"
