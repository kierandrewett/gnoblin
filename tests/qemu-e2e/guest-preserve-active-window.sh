#!/usr/bin/env bash
# Guest: layer_shell.preserve_active_window keeps the focused window focused when a layer-shell launcher opens.
#
# A foot window is clicked so it is focused, then fuzzel (a layer-shell launcher with keyboard focus) opens. With the
# setting true the foot window stays focused. With false the layer surface takes focus, so no window is focused.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-preserve.lua"
CLICK="${GNOBLIN_CLICK:-/tmp/guest-click.py}"
fail=0

check() {
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

focused() {
    "$G" window list | python3 -c '
import json, sys
print(",".join(w["title"] for w in json.load(sys.stdin)["windows"] if w.get("focused")) or "none")'
}
center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "preserve-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}

for mode in true false; do
    printf 'gnoblin.configure {layer_shell = {preserve_active_window = %s}}\n' "$mode" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    nohup foot -T preserve-test >/dev/null 2>&1 < /dev/null &
    sleep 4
    python3 "$CLICK" $(center) >/dev/null 2>&1
    sleep 1
    before="$(focused)"
    nohup sh -c 'printf "one\ntwo\n" | fuzzel --dmenu' >/dev/null 2>&1 < /dev/null &
    sleep 3
    after="$(focused)"
    pkill -x fuzzel
    pkill -f "foot -T preserve-test"
    sleep 2
    check "preserve_active_window $mode: the foot window is focused before the launcher opens" "$before" "preserve-test"
    if [ "$mode" = true ]; then
        check "preserve_active_window true keeps the foot window focused" "$after" "preserve-test"
    else
        check "preserve_active_window false lets the launcher take focus" "$after" "none"
    fi
done

rm -f "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
