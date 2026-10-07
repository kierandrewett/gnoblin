#!/usr/bin/env bash
# Guest: report which window has focus and how bright each one is, then clean up.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
sleep 1
grim -t ppm /tmp/unfocused.ppm
"$G" window list 2>&1 | python3 -c '
import json, sys
data = open("/tmp/unfocused.ppm", "rb").read()
parts = data.split(b"\n", 3)
width = int(parts[1].split()[0])
for w in json.load(sys.stdin)["windows"]:
    if w["title"].startswith("dim-"):
        f = w["frame"]
        # dim-a is raised by the click, so its centre is visible; dim-b shows a strip on its right side
        if w["title"] == "dim-a":
            x, y = f["x"] + f["width"] // 2, f["y"] + f["height"] // 2
        else:
            x, y = f["x"] + f["width"] - 20, f["y"] + 300
        print("RESULT", w["title"], "focused", w["focused"], "red", parts[3][(y * width + x) * 3])'
pkill -x swaybg
pkill -f "foot -T dim-"
printf -- "-- disabled\n" > "$HOME/.config/gnoblin/config/99-test-unfocused.lua"
"$G" config reload | tail -1
