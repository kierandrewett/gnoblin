#!/usr/bin/env bash
# Guest: window_management.constrain_drag_to_work_area keeps a dragged window out of a panel's exclusive zone, or not.
#
# Stock Mutter already keeps a titlebar inside the work area at the top, so an upward drag cannot tell the settings
# apart. At the bottom stock Mutter lets a window leave all but CLAMP(height/4, 10, 75) px past the edge. With the
# setting true the whole window must stay inside the work area; with false the stock rule applies. Waybar is running so
# the work area is not the whole monitor. Edge tiling is off so the drag cannot tile the window.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-dragconstraint.lua"
DRAG="${GNOBLIN_TITLEBAR_DRAG:-/tmp/guest-titlebar-drag.py}"
fail=0

check() {
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

frame() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "drag-constraint":
        f = w["frame"]
        print("%d %d %d %d" % (f["x"], f["y"], f["width"], f["height"]))'
}

nohup waybar >/dev/null 2>&1 < /dev/null &
sleep 5
mon_h="$("$G" monitor list | python3 -c 'import json, sys; print(json.load(sys.stdin)["monitors"][0]["height"])')"
for mode in true false; do
    apply "gnoblin.configure {window_management = {edge_tiling = false, constrain_drag_to_work_area = $mode}}"
    nohup foot -T drag-constraint >/dev/null 2>&1 < /dev/null &
    sleep 4
    read -r wx wy ww wh <<<"$(frame)"
    python3 "$DRAG" $((wx + ww / 2)) $((wy + 12)) 0 $((mon_h)) >/dev/null 2>&1
    sleep 2
    read -r nx ny nw nh <<<"$(frame)"
    pkill -f "foot -T drag-constraint"
    sleep 2
    echo "info constrain_drag_to_work_area=$mode before_y=$wy after_y=$ny"
    if [ "$mode" = true ]; then
        check "constrain true: the whole window stays inside the work area" "$([ $((ny + nh)) -le "$mon_h" ] && echo yes || echo no)" "yes"
    else
        check "constrain false: the stock rule lets the window leave the work area" "$([ $((ny + nh)) -gt "$mon_h" ] && echo yes || echo no)" "yes"
    fi
done
pkill -x waybar
rm -f "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
