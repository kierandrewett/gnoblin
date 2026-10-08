#!/usr/bin/env bash
# Guest: two config sections change behaviour on reload without a new session.
#
# autostart: adding an entry starts its command, enable = false stops it, enabling it again starts it again,
#            and removing the config stops it.
# window_management.focus_mode: "click" keeps focus when the pointer moves onto another window, "sloppy"
#            focuses the window under the pointer.
# window_management.raise_on_click: a click raises the window above an overlapping one, or leaves the stacking.
# window_management.edge_tiling: dragging a titlebar to the left screen edge tiles the window to the left half, or
#            only moves it.
# window_management.center_new_windows: true opens a new window centred on the monitor, false uses Mutter's
#            default placement.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
F="$HOME/.config/gnoblin/config/99-test-reload-effects.lua"
fail=0

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

apply() {
    printf '%s\n' "$1" > "$F"
    "$G" config reload >/dev/null
    sleep 4
}

marker_count() { pgrep -fc 'gnoblin-autostart-marker-sleep' 2>/dev/null || true; }

ENTRY='{command = {"sh", "-c", "exec -a gnoblin-autostart-marker-sleep sleep 600"}'
check "no marker process before the test" "$(marker_count)" "0"
apply "gnoblin.configure {autostart = {marker = $ENTRY}}}"
check "adding an autostart entry starts it" "$(marker_count)" "1"
apply "gnoblin.configure {autostart = {marker = ${ENTRY}, enable = false}}}"
check "enable = false stops it" "$(marker_count)" "0"
apply "gnoblin.configure {autostart = {marker = $ENTRY}}}"
check "enabling it again starts it again" "$(marker_count)" "1"
apply '-- disabled'
check "removing the config stops it" "$(marker_count)" "0"

cat > /tmp/pointer-move.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

x, y = float(sys.argv[1]), float(sys.argv[2])
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
steps = 10
for _ in range(steps):
    call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (x / steps, y / steps)))
    time.sleep(0.05)
time.sleep(0.6)
call(session, iface, "Stop", None)
PY

focused() {
    "$G" window list | python3 -c '
import json, sys
print(",".join(w["title"] for w in json.load(sys.stdin)["windows"] if w.get("focused")) or "none")'
}
window_id() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        print(w["id"])'
}
center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}

nohup swaybg -c "#303030" >/dev/null 2>&1 < /dev/null &
nohup foot -T focus-A >/dev/null 2>&1 < /dev/null &
sleep 3
nohup foot -T focus-B >/dev/null 2>&1 < /dev/null &
sleep 3
"$G" window move "$(window_id focus-A)" 40 100 >/dev/null 2>&1
"$G" window move "$(window_id focus-B)" 760 100 >/dev/null 2>&1
sleep 1

for mode in click sloppy; do
    apply "gnoblin.configure {window_management = {focus_mode = \"$mode\"}}"
    python3 "$CLICK" $(center focus-A) >/dev/null 2>&1
    sleep 1
    before="$(focused)"
    python3 /tmp/pointer-move.py $(center focus-B)
    sleep 1
    after="$(focused)"
    if [ "$mode" = click ]; then
        check "focus_mode click keeps focus when the pointer enters another window" "$before -> $after" "focus-A -> focus-A"
    else
        check "focus_mode sloppy focuses the window under the pointer" "$before -> $after" "focus-A -> focus-B"
    fi
done

pkill -f "foot -T focus-"
pkill -x swaybg
sleep 1

# window_management.center_new_windows: a new window opens centred on the first monitor, or at Mutter's default spot.
monitor_center="$("$G" monitor list | python3 -c '
import json, sys
data = json.load(sys.stdin)
monitors = data.get("monitors", data) if isinstance(data, dict) else data
m = monitors[0]
box = m.get("rect") or m.get("geometry") or m
x, y = m.get("x", box.get("x", 0)), m.get("y", box.get("y", 0))
w, h = m.get("width", box.get("width")), m.get("height", box.get("height"))
print(x + w // 2, y + h // 2)')"
new_window_center() {
    nohup foot -T center-test >/dev/null 2>&1 < /dev/null &
    sleep 4
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "center-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
    pkill -f "foot -T center-test"
    sleep 1
}
distance() {
    python3 -c 'import sys; a = list(map(int, sys.argv[1:5])); print(max(abs(a[0] - a[2]), abs(a[1] - a[3])))' $1 $2
}
apply 'gnoblin.configure {window_management = {center_new_windows = true}}'
centered="$(distance "$(new_window_center)" "$monitor_center")"
apply 'gnoblin.configure {window_management = {center_new_windows = false}}'
default_spot="$(distance "$(new_window_center)" "$monitor_center")"
if [ "$centered" -le 2 ]; then
    echo "PASS center_new_windows true centres the new window (off by $centered px)"
else
    echo "FAIL center_new_windows true is off by $centered px"
    fail=$((fail + 1))
fi
if [ "$default_spot" -gt 10 ]; then
    echo "PASS center_new_windows false uses the default placement (off centre by $default_spot px)"
else
    echo "FAIL center_new_windows false still centres the window (off by $default_spot px)"
    fail=$((fail + 1))
fi

# window_management.raise_on_click: clicking a window raises it above an overlapping one, or does not.
python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1
overlap_pixel() {
    grim /tmp/raise-shot.png
    python3 -c 'from PIL import Image; print(Image.open("/tmp/raise-shot.png").convert("RGB").getpixel((450, 300)))'
}
for mode in true false; do
    apply "gnoblin.configure {window_management = {raise_on_click = $mode}}"
    nohup swaybg -c "#202020" >/dev/null 2>&1 < /dev/null &
    nohup foot -T raise-A -o colors.background=ff0000 -o colors-dark.background=ff0000 -o colors-light.background=ff0000 >/dev/null 2>&1 < /dev/null &
    sleep 3
    nohup foot -T raise-B -o colors.background=0000ff -o colors-dark.background=0000ff -o colors-light.background=0000ff >/dev/null 2>&1 < /dev/null &
    sleep 3
    "$G" window move "$(window_id raise-A)" 40 100 >/dev/null 2>&1
    "$G" window move "$(window_id raise-B)" 300 160 >/dev/null 2>&1
    sleep 1
    before="$(overlap_pixel)"
    python3 "$CLICK" 80 300 >/dev/null 2>&1
    sleep 1
    after="$(overlap_pixel)"
    pkill -f "foot -T raise-"
    pkill -x swaybg
    sleep 2
    check "raise_on_click $mode: overlap before the click is the newer window (blue)" "$before" "(0, 0, 255)"
    if [ "$mode" = true ]; then
        check "raise_on_click true raises the clicked window (overlap turns red)" "$after" "(255, 0, 0)"
    else
        check "raise_on_click false leaves the stacking alone (overlap stays blue)" "$after" "(0, 0, 255)"
    fi
done

# window_management.edge_tiling: dragging a titlebar to the left screen edge tiles the window, or does not.
DRAG="${GNOBLIN_TITLEBAR_DRAG:-/tmp/guest-titlebar-drag.py}"
tile_frame() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "tile-test":
        f = w["frame"]
        print("%d %d %d %d" % (f["x"], f["y"], f["width"], f["height"]))'
}
monitor_size="$("$G" monitor list | python3 -c '
import json, sys
data = json.load(sys.stdin)
monitors = data.get("monitors", data) if isinstance(data, dict) else data
m = monitors[0]
box = m.get("rect") or m.get("geometry") or m
print(m.get("width", box.get("width")), m.get("height", box.get("height")))')"
read -r mon_w mon_h <<<"$monitor_size"
for mode in true false; do
    apply "gnoblin.configure {window_management = {edge_tiling = $mode}}"
    nohup foot -T tile-test >/dev/null 2>&1 < /dev/null &
    sleep 4
    read -r wx wy ww wh <<<"$(tile_frame)"
    python3 "$DRAG" $((wx + ww / 2)) $((wy + 12)) -$((wx + ww / 2 + 200)) 0 >/dev/null 2>&1
    sleep 2
    read -r nx ny nw nh <<<"$(tile_frame)"
    pkill -f "foot -T tile-test"
    sleep 2
    if [ "$mode" = true ]; then
        check "edge_tiling true tiles the window to the left half" "$nx $nw $nh" "0 $((mon_w / 2)) $mon_h"
    else
        check "edge_tiling false only moves the window (size unchanged)" "$nw $nh" "$ww $wh"
    fi
done

apply '-- disabled'
rm -f "$F"
echo "failures: $fail"
exit "$fail"
