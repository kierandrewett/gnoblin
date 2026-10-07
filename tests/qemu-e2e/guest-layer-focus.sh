#!/usr/bin/env bash
# Guest: layer_shell.preserve_active_window decides whether a keyboard-interactive layer surface
# (fuzzel) takes focus from the focused app window (foot).
#
# true  -> the app window stays focused after the layer surface opens
# false -> focus moves to the layer surface, so no toplevel is focused
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
F="$HOME/.config/gnoblin/config/99-test-layer.lua"
fail=0

focused() {
    "$G" window list | python3 -c '
import json, sys
print(",".join(w["title"] for w in json.load(sys.stdin)["windows"] if w.get("focused")) or "none")'
}

run_case() {
    printf 'gnoblin.configure {layer_shell = {preserve_active_window = %s}}\n' "$1" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    nohup foot -T layer-test >/dev/null 2>&1 < /dev/null &
    sleep 4
    center="$("$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "layer-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)')"
    python3 "$CLICK" $center >/dev/null 2>&1
    sleep 2
    nohup fuzzel >/dev/null 2>&1 < /dev/null &
    sleep 4
    result="$(focused)"
    pkill -x fuzzel
    pkill -f "foot -T layer-test"
    sleep 2
    if [ "$result" = "$2" ]; then
        echo "PASS preserve_active_window=$1 -> focused: $result"
    else
        echo "FAIL preserve_active_window=$1 -> focused: $result (expected $2)"
        fail=$((fail + 1))
    fi
}

run_case true layer-test
run_case false none
printf -- '-- disabled\n' > "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
