#!/usr/bin/env bash
# Guest: Super + left-drag a focused window to the right edge and verify Bingux commits a snap target.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
DRAG="${GNOBLIN_DRAG_SCRIPT:-/tmp/guest-resize-drag.py}"
CONFIG="$HOME/.config/gnoblin/config/99-test-snap-assist.lua"
CONFIG_DIR="$(dirname "$CONFIG")"
foot_pid=
config_dir_created=0

cleanup() {
    if [ -n "$foot_pid" ]; then
        kill "$foot_pid" 2>/dev/null || true
        wait "$foot_pid" 2>/dev/null || true
    fi
    rm -f "$CONFIG"
    if [ "$config_dir_created" -eq 1 ]; then
        rmdir "$CONFIG_DIR" 2>/dev/null || true
    fi
    "$G" config reload >/dev/null 2>&1 || true
}
frame() {
    "$G" window list | python3 -c '
import json, sys
w = next(w for w in json.load(sys.stdin)["windows"] if w["title"] == "snap-assist-test")
f = w["frame"]
print(f["x"], f["y"], f["width"], f["height"], int(w["focused"]))'
}
trap cleanup EXIT

if [ -e "$CONFIG" ]; then
    echo "FAIL refusing to overwrite existing test config: $CONFIG"
    exit 1
fi
if [ ! -d "$CONFIG_DIR" ]; then
    mkdir -p "$CONFIG_DIR"
    config_dir_created=1
fi
printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$CONFIG"
if ! "$G" config reload; then
    echo "FAIL could not reload test configuration using $G"
    exit 1
fi
sleep 2
foot -T snap-assist-test >/dev/null 2>&1 </dev/null &
foot_pid=$!
sleep 4
read -r x y width height focused <<<"$(frame)"
echo "before snap: x=$x y=$y width=$width height=$height focused=$focused"
[ "$focused" -eq 1 ] || {
    echo "FAIL test window did not receive focus"
    exit 1
}

monitor_width=$("$G" monitor list | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["monitors"][0]["width"])')
start_x=$((x + width / 2))
start_y=$((y + 12))
delta_x=$((monitor_width - start_x + 120))
python3 "$DRAG" "$start_x" "$start_y" "$delta_x" 0 left
sleep 1
read -r after_x after_y after_width after_height after_focused <<<"$(frame)"
echo "after right-edge snap: x=$after_x y=$after_y width=$after_width height=$after_height focused=$after_focused"
if ((after_x + after_width >= monitor_width - 20 && after_width < monitor_width && (\
    after_x != x || after_y != y || after_width != width || after_height != height))); then
    echo "PASS Bingux SnapAssist placed the window against the right edge"
else
    echo "FAIL Bingux SnapAssist did not commit a right-edge target"
    exit 1
fi
