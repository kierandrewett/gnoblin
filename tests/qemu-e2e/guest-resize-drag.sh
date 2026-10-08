#!/usr/bin/env bash
# Guest: Super + right-button drag resizes from the corner nearest the pointer.
#
# Opens a foot window and drags from each quadrant in turn: south-east, north-west, north-east, south-west. It prints
# the frame before and after each drag. Each drag must move the corner nearest the pointer and keep the opposite
# edges where they were.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
HERE="$(dirname "$(readlink -f "$0")")"
DRAG="${GNOBLIN_DRAG_SCRIPT:-/tmp/guest-resize-drag.py}"
fail=0

frame() {
    "$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "resize-test":
        f = w["frame"]
        print(f["x"], f["y"], f["width"], f["height"])'
}

nohup foot -T resize-test >/dev/null 2>&1 < /dev/null &
sleep 4
read -r x y w h <<< "$(frame)"
echo "start frame: x=$x y=$y w=$w h=$h"
[ -n "${w:-}" ] || { echo "FAIL window did not appear"; exit 1; }

# South-east quadrant, drag right and down.
python3 "$DRAG" $((x + w * 3 / 4)) $((y + h * 3 / 4)) 120 80
sleep 1
read -r x2 y2 w2 h2 <<< "$(frame)"
echo "after south-east drag: x=$x2 y=$y2 w=$w2 h=$h2"
if [ "$x2" -eq "$x" ] && [ "$y2" -eq "$y" ] && [ "$w2" -gt "$w" ] && [ "$h2" -gt "$h" ]; then
    echo "PASS south-east drag grows the window and keeps the top-left corner"
else
    echo "FAIL south-east drag"
    fail=$((fail + 1))
fi

# North-west quadrant, drag left and up.
python3 "$DRAG" $((x2 + w2 / 4)) $((y2 + h2 / 4)) -60 -40
sleep 1
read -r x3 y3 w3 h3 <<< "$(frame)"
echo "after north-west drag: x=$x3 y=$y3 w=$w3 h=$h3"
right2=$((x2 + w2))
right3=$((x3 + w3))
if [ "$x3" -lt "$x2" ] && [ "$y3" -lt "$y2" ] && [ "$right3" -eq "$right2" ]; then
    echo "PASS north-west drag moves the top-left corner and keeps the right edge"
else
    echo "FAIL north-west drag"
    fail=$((fail + 1))
fi

# North-east quadrant, drag right and up. The left and bottom edges stay; the top edge moves up and the width grows.
python3 "$DRAG" $((x3 + w3 * 3 / 4)) $((y3 + h3 / 4)) 50 -40
sleep 1
read -r x4 y4 w4 h4 <<< "$(frame)"
echo "after north-east drag: x=$x4 y=$y4 w=$w4 h=$h4"
if [ "$x4" -eq "$x3" ] && [ "$y4" -lt "$y3" ] && [ "$w4" -gt "$w3" ] && [ "$((y4 + h4))" -eq "$((y3 + h3))" ]; then
    echo "PASS north-east drag moves the top edge and keeps the left and bottom edges"
else
    echo "FAIL north-east drag"
    fail=$((fail + 1))
fi

# South-west quadrant, drag left and down. The top and right edges stay; the left edge moves out and the height grows.
python3 "$DRAG" $((x4 + w4 / 4)) $((y4 + h4 * 3 / 4)) -50 40
sleep 1
read -r x5 y5 w5 h5 <<< "$(frame)"
echo "after south-west drag: x=$x5 y=$y5 w=$w5 h=$h5"
if [ "$x5" -lt "$x4" ] && [ "$y5" -eq "$y4" ] && [ "$h5" -gt "$h4" ] && [ "$((x5 + w5))" -eq "$((x4 + w4))" ]; then
    echo "PASS south-west drag moves the left edge and keeps the top and right edges"
else
    echo "FAIL south-west drag"
    fail=$((fail + 1))
fi

pkill -f "foot -T resize-test"
echo "failures: $fail"
exit "$fail"
