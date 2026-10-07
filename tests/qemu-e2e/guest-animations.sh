#!/usr/bin/env bash
# Guest: does a normal window fade in, and does a dialog fade in? Samples shortly after each opens.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-animations.lua"

sample_window() {
    # sample_window TITLE PPM: print the red channel at the centre of the named window
    local geometry
    geometry="$("$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == sys.argv[1]:
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)' "$1")"
    [ -n "$geometry" ] || { echo "no window titled $1"; return; }
    python3 - "$2" $geometry <<'PY'
import sys
path, x, y = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
data = open(path, "rb").read()
parts = data.split(b"\n", 3)
width = int(parts[1].split()[0])
print(parts[3][(y * width + x) * 3])
PY
}

cp /tmp/99-test-animations.lua "$CONFIG"
"$G" config reload | tail -1
sleep 2
nohup swaybg -c "#ffffff" >/dev/null 2>&1 < /dev/null &
sleep 1

echo "== normal window, sampled 0.8 s after launch (a 3 s fade from transparent)"
nohup foot -T anim-normal >/dev/null 2>&1 < /dev/null &
sleep 0.8
grim -t ppm /tmp/anim-normal.ppm
normal_red="$(sample_window anim-normal /tmp/anim-normal.ppm)"
echo "normal window red channel: $normal_red  (36 = already opaque, near 200 = fading in)"
sleep 4

echo "== modal dialog, sampled 0.8 s after it appears"
nohup python3 /tmp/dialog-test.py >/tmp/dialog-test.log 2>&1 < /dev/null &
for _ in $(seq 1 150); do
    grep -q "dialog presented" /tmp/dialog-test.log 2>/dev/null && break
    sleep 0.1
done
sleep 0.8
grim -t ppm /tmp/anim-dialog.ppm
# The dialog is not listed by gnoblinctl, so sample the centre of its parent, which the dialog covers.
dialog_red="$(sample_window dialog-parent /tmp/anim-dialog.ppm)"
echo "dialog red channel: $dialog_red  (32 = dark dialog already opaque, above 100 = fading in)"

if [ "$normal_red" -gt 100 ] 2>/dev/null; then
    echo "PASS open animation fades a normal window in ($normal_red)"
else
    echo "FAIL open animation fades a normal window in ($normal_red)"
fi
if [ "$dialog_red" -gt 100 ] 2>/dev/null; then
    echo "PASS dialog-open animation fades a dialog in ($dialog_red)"
else
    echo "FAIL dialog-open animation fades a dialog in ($dialog_red)"
fi
pkill -f dialog-test.py
pkill -f "foot -T anim-normal"
pkill -x swaybg
printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload | tail -1
