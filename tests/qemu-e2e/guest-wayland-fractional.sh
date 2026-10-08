#!/usr/bin/env bash
# Guest: a native Wayland GTK4 app keeps its logical size at a fractional monitor scale and sees the fractional scale.
#
# Both virtual monitors are set to 1920x1080 at scale 1.0, then at scale 1.5 (a 1280x720 logical size). The compositor's
# frame size of a 400x300 window must be the same at both scales, and GTK must report the fractional scale 1.5, which
# the fractional-scale protocol carries. The monitors are restored to their preferred mode at scale 1.0 at the end.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
GD="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gdctl"
G="${GNOBLIN_PREFIX}/bin/gnoblinctl"
MODE="${GNOBLIN_TEST_MODE:-1920x1080@60.000}"
SCALE="${GNOBLIN_TEST_SCALE:-1.5}"
fail=0

cat > /tmp/wl-scale-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="wl-scale-test")
window.set_default_size(400, 300)
window.set_child(Gtk.Label(label="size test"))
window.present()
loop = GLib.MainLoop()


def report():
    surface = window.get_surface()
    open("/tmp/wl-scale.txt", "w").write("fractional=%s\n" % surface.get_scale())
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
    # Prints "WIDTHxHEIGHT fractional=SCALE" for the test window.
    sleep 3
    rm -f /tmp/wl-scale.txt
    GDK_BACKEND=wayland timeout 12 python3 /tmp/wl-scale-window.py >/dev/null 2>&1 &
    sleep 4.5
    frame="$("$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "wl-scale-test":
        f = w["frame"]
        print("%dx%d" % (f["width"], f["height"]))')"
    wait
    echo "$frame $(cat /tmp/wl-scale.txt 2>/dev/null)"
}

set_scale 1 "$MODE"
base="$(measure)"
set_scale "$SCALE" "$MODE"
fractional="$(measure)"
set_scale 1
sleep 3
restored="$("$GD" show 2>&1 | grep -m1 -o 'Scale: [0-9.]*')"
mode_after="$("$GD" show 2>&1 | grep -m1 -o '[0-9]*x[0-9]*@[0-9.]*')"

echo "scale 1.0:  $base"
echo "scale $SCALE: $fractional"
if [ "${base%% *}" = "${fractional%% *}" ] && [ -n "${base%% *}" ]; then
    echo "PASS the Wayland window keeps its logical size (${base%% *}) at scale $SCALE"
else
    echo "FAIL the Wayland window changed size at scale $SCALE (1.0: ${base%% *}, $SCALE: ${fractional%% *})"
    fail=$((fail + 1))
fi
case "$base" in
    *"fractional=1.0"*) echo "PASS the app sees scale 1.0 before the change" ;;
    *) echo "FAIL the app scale is wrong at monitor scale 1.0 ($base)"; fail=$((fail + 1)) ;;
esac
case "$fractional" in
    *"fractional=$SCALE"*) echo "PASS the app sees the fractional scale $SCALE" ;;
    *) echo "FAIL the app does not see the fractional scale $SCALE ($fractional)"; fail=$((fail + 1)) ;;
esac
case "$restored" in
    *"Scale: 1.0"*) echo "PASS the monitor layout is back at scale 1.0 ($mode_after)" ;;
    *) echo "FAIL the monitor layout was not restored ($restored)"; fail=$((fail + 1)) ;;
esac
echo "failures: $fail"
exit "$fail"
