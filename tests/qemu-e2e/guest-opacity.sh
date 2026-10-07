#!/usr/bin/env bash
# Guest: sample a window pixel over a white wallpaper before and after an opacity rule.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-opacity.lua"

sample() {
    # sample LABEL: print the RGB pixel at the centre of the opacity-test window
    local geometry
    geometry="$("$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "opacity-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)')"
    grim -t ppm /tmp/opacity.ppm
    python3 - "$1" $geometry <<'PY'
import sys
label, x, y = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
data = open("/tmp/opacity.ppm", "rb").read()
parts = data.split(b"\n", 3)
width, height = map(int, parts[1].split())
pixels = parts[3]
offset = (y * width + x) * 3
print(label, "pixel at", x, y, "=", tuple(pixels[offset:offset + 3]))
PY
}

nohup swaybg -c "#ffffff" >/dev/null 2>&1 < /dev/null &
nohup foot -T opacity-test >/dev/null 2>&1 < /dev/null &
sleep 4
sample "without rule"
cp /tmp/99-test-opacity.lua "$CONFIG"
"$G" config reload | tail -1
sleep 3
sample "with opacity 0.4 rule"
pkill -x swaybg
pkill -f "foot -T opacity-test"
printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload | tail -1
