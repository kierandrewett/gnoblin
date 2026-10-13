#!/usr/bin/env bash
# Guest: a background layer surface follows a change of the output mode.
#
# swaybg is the background layer surface (anchored to all four edges, size 0x0, so the compositor
# chooses the size). Mutter leaves desktop-type windows alone when the monitor layout changes, and
# background and bottom layer surfaces are desktop windows, so they used to keep the size of the
# previous mode. The output is switched to a smaller and then to the original mode; the surface
# must match the monitor each time. The original mode is restored at the end.
# shellcheck disable=SC2155
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
fail=0

set_mode() { # set_mode WIDTHxHEIGHT; with no argument, print the current mode id
    python3 - "$@" <<'PY'
import sys
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
def call(method, args=None):
    return bus.call_sync("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
                         "org.gnome.Mutter.DisplayConfig", method, args, None, Gio.DBusCallFlags.NONE, 5000, None)
serial, monitors, logical, _ = call("GetCurrentState").unpack()
monitor = monitors[0]
current = next(m for m in monitor[1] if m[6].get("is-current"))
if len(sys.argv) == 1:
    print("%dx%d" % (current[1], current[2]))
    raise SystemExit
width, height = (int(v) for v in sys.argv[1].split("x"))
mode = next(m[0] for m in monitor[1] if m[1] == width and m[2] == height)
config = GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})",
                      (serial, 1, [(0, 0, 1.0, 0, True, [(monitor[0][0], mode, {})])], {}))
call("ApplyMonitorsConfig", config)
PY
}

# Prints "MONITORWIDTHxHEIGHT SURFACEWIDTHxHEIGHT" for the swaybg surface (a background layer surface in the namespace wallpaper).
sizes() {
    "$G" --json layer list | python3 -c '
import json, subprocess, sys
layers = json.load(sys.stdin)["layers"]
monitor = json.loads(subprocess.check_output(["'"$G"'", "--json", "monitor", "list"]))["monitors"][0]
bg = [l for l in layers if l["layer"] == "background" and l.get("namespace") == "wallpaper"]
print("%dx%d %s" % (monitor["width"], monitor["height"],
      ",".join("%dx%d" % (l["geometry"]["width"], l["geometry"]["height"]) for l in bg) or "none"))'
}

check() { # check NAME ACTUAL
    set -- "$1" "$2"
    local monitor="${2%% *}" surface="${2#* }"
    if [ "$monitor" = "$surface" ]; then
        echo "PASS $1 ($2)"
    else
        echo "FAIL $1 (monitor and background surface: $2)"
        fail=$((fail + 1))
    fi
}

original="$(set_mode)"
trap 'pkill -x swaybg 2>/dev/null; set_mode "$original" >/dev/null 2>&1' EXIT

nohup swaybg -c '#336699' >/tmp/swaybg.log 2>&1 </dev/null &
sleep 3
check "the background surface covers the monitor at the original mode $original" "$(sizes)"

for mode in 1024x768 "$original" 1024x768 "$original"; do
    set_mode "$mode" >/dev/null
    sleep 3
    check "the background surface follows the output to $mode" "$(sizes)"
done

[ "$fail" -eq 0 ] && echo "ALL PASS" || echo "$fail FAILED"
exit "$fail"
