#!/usr/bin/env bash
# Guest: Super + right-button drag resizes from the corner nearest the pointer.
#
# Opens a foot window, drags from its south-east quadrant, then from its north-west quadrant, and
# prints the frame before and after each drag. The south-east drag must grow the window and keep
# its top-left corner. The north-west drag must move the top-left corner and keep the bottom-right.
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

pkill -f "foot -T resize-test"
echo "failures: $fail"
exit "$fail"
