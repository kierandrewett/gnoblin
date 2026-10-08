#!/usr/bin/env bash
# Guest: window_management.action_double_click_titlebar changes what a double click on a titlebar does.
#
# Each mode is applied with a config reload, a foot window gets a double click on its titlebar, and the window's
# maximized and minimized state is read back. The click position is the titlebar line used by the drag tests
# (12 px below the top of the frame).
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

rm -f "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
