#!/usr/bin/env bash
# Guest: dragging a titlebar to a screen edge tiles or maximizes the window, as in GNOME.
#
# With edge_tiling on: the right edge tiles the window to the right half, the top edge maximizes it, and dragging a
# tiled window away from the edge gives it back its size from before the tile. (The left edge is covered by
# run-reload-effects.sh.) Each case starts a fresh window and drags its titlebar with a real left button through
# RemoteDesktop.
#
# The guest has two monitors side by side, so the right edge of the first is not a screen edge. The test switches to
# the first monitor alone and restores both at the end.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
GD="${GNOBLIN_PREFIX}/bin/gdctl"
DRAG="${GNOBLIN_TITLEBAR_DRAG:-/tmp/guest-titlebar-drag.py}"
F="$HOME/.config/gnoblin/config/99-test-edge-tiling.lua"
fail=0
restore_monitors() {
    "$GD" set --logical-monitor --primary --monitor Virtual-1 --scale 1 \
        --logical-monitor --monitor Virtual-2 --scale 1 --right-of Virtual-1 >/dev/null 2>&1
}
trap 'rm -f "$F"; pkill -f "foot -T edge-test" 2>/dev/null; restore_monitors; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

printf 'gnoblin.configure {window_management = {edge_tiling = true}}\n' > "$F"
"$G" config reload >/dev/null 2>&1
"$GD" set --logical-monitor --primary --monitor Virtual-1 --scale 1 >/dev/null 2>&1
sleep 4

# Prints "X Y W H MAXIMIZED" for the test window.
frame() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "edge-test":
        f = w["frame"]
        print("%d %d %d %d %s" % (f["x"], f["y"], f["width"], f["height"], w["maximized"]))'
}
read -r mon_w mon_h <<<"$("$G" monitor list | python3 -c '
import json, sys
data = json.load(sys.stdin)
monitors = data.get("monitors", data) if isinstance(data, dict) else data
m = monitors[0]
box = m.get("rect") or m.get("geometry") or m
print(m.get("width", box.get("width")), m.get("height", box.get("height")))')"

start_window() {
    nohup foot -T edge-test >/dev/null 2>&1 < /dev/null &
    sleep 4
}
stop_window() { pkill -f "foot -T edge-test"; sleep 2; }

# Right edge.
start_window
read -r wx wy ww wh wmax <<<"$(frame)"
python3 "$DRAG" $((wx + ww / 2)) $((wy + 12)) $((mon_w - wx - ww / 2 + 200)) 0 >/dev/null 2>&1
sleep 2
read -r rx ry rw rh rmax <<<"$(frame)"
stop_window
check "dragging to the right edge tiles the window to the right half" "$rx $rw $rh" "$((mon_w / 2)) $((mon_w / 2)) $mon_h"

# Top edge.
start_window
read -r wx wy ww wh wmax <<<"$(frame)"
python3 "$DRAG" $((wx + ww / 2)) $((wy + 12)) 0 -$((wy + 12 + 200)) >/dev/null 2>&1
sleep 2
read -r tx ty tw th tmax <<<"$(frame)"
stop_window
check "dragging to the top edge maximizes the window" "$tmax" "True"

# Drag a tiled window away from the edge: it must get its earlier size back.
start_window
read -r wx wy ww wh wmax <<<"$(frame)"
python3 "$DRAG" $((wx + ww / 2)) $((wy + 12)) -$((wx + ww / 2 + 200)) 0 >/dev/null 2>&1
sleep 2
read -r lx ly lw lh lmax <<<"$(frame)"
check "the window is tiled to the left half first" "$lx $lw" "0 $((mon_w / 2))"
python3 "$DRAG" $((lx + lw / 2)) $((ly + 12)) 300 300 >/dev/null 2>&1
sleep 2
read -r ax ay aw ah amax <<<"$(frame)"
stop_window
check "dragging a tiled window away gives it back its size" "$aw $ah" "$ww $wh"
check "the window is no longer tiled or maximized after the drag away" "$([ "$ax" -gt 0 ] && echo moved):$amax" "moved:False"

echo "failures: $fail"
exit "$fail"
