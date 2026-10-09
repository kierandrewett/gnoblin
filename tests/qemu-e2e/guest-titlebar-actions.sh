#!/usr/bin/env bash
# Guest: window_management titlebar actions change what a click on a titlebar does.
#
# Double click: each mode is applied with a config reload, a foot window gets a double click on its titlebar, and the
# window's maximized and minimized state is read back. Middle click: two overlapping coloured windows, a middle click
# on the upper one's titlebar, and the colour at the overlap says which window is on top. The click position is the
# titlebar line used by the drag tests (12 px below the top of the frame).
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-titlebar.lua"
DOUBLE="${GNOBLIN_DOUBLE_CLICK:-/tmp/guest-double-click.py}"
fail=0

check() {
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

state() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "titlebar-test":
        print("maximized=%s minimized=%s" % (str(w["maximized"]).lower(), str(w["minimized"]).lower()))'
}
frame() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "titlebar-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + 12)'
}

for case in "toggle-maximize|maximized=true minimized=false" "minimize|maximized=false minimized=true" "none|maximized=false minimized=false"; do
    mode="${case%%|*}"
    expected="${case#*|}"
    printf 'gnoblin.configure {window_management = {action_double_click_titlebar = "%s"}}\n' "$mode" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    nohup foot -T titlebar-test >/dev/null 2>&1 < /dev/null &
    sleep 4
    python3 "$DOUBLE" $(frame) >/dev/null 2>&1
    sleep 2
    check "action_double_click_titlebar = $mode" "$(state)" "$expected"
    pkill -f "foot -T titlebar-test"
    sleep 2
done

# action_middle_click_titlebar: "lower" sends the clicked window behind the one it overlaps, "none" leaves it.
CLICK_BUTTON="${GNOBLIN_CLICK_BUTTON:-/tmp/guest-click-button.py}"
python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1
overlap_pixel() {
    grim /tmp/titlebar-shot.png
    python3 -c 'from PIL import Image; print(Image.open("/tmp/titlebar-shot.png").convert("RGB").getpixel((450, 300)))'
}
window_id() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        print(w["id"])'
}
for case in "lower|(255, 0, 0)" "none|(0, 0, 255)"; do
    mode="${case%%|*}"
    expected="${case#*|}"
    printf 'gnoblin.configure {window_management = {action_middle_click_titlebar = "%s"}}\n' "$mode" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    nohup swaybg -c "#202020" >/dev/null 2>&1 < /dev/null &
    nohup foot -T tb-A -o colors.background=ff0000 -o colors-dark.background=ff0000 -o colors-light.background=ff0000 >/dev/null 2>&1 < /dev/null &
    sleep 3
    nohup foot -T tb-B -o colors.background=0000ff -o colors-dark.background=0000ff -o colors-light.background=0000ff >/dev/null 2>&1 < /dev/null &
    sleep 3
    "$G" window move "$(window_id tb-A)" 40 100 >/dev/null 2>&1
    "$G" window move "$(window_id tb-B)" 300 160 >/dev/null 2>&1
    sleep 1
    before="$(overlap_pixel)"
    python3 "$CLICK_BUTTON" middle 650 172 >/dev/null 2>&1
    sleep 1
    after="$(overlap_pixel)"
    pkill -f "foot -T tb-"
    pkill -x swaybg
    sleep 2
    check "action_middle_click_titlebar = $mode: the overlap starts as the newer window (blue)" "$before" "(0, 0, 255)"
    check "action_middle_click_titlebar = $mode: the overlap after a middle click on its titlebar" "$after" "$expected"
done

rm -f "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
