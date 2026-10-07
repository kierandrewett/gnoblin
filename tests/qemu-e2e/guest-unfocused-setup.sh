#!/usr/bin/env bash
# Guest: install the unfocused-dimming rule and open two windows. Prints the window centres.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
cp /tmp/99-test-unfocused.lua "$HOME/.config/gnoblin/config/99-test-unfocused.lua"
"$G" config reload | tail -1
nohup swaybg -c "#ffffff" >/dev/null 2>&1 < /dev/null &
nohup foot -T dim-a >/dev/null 2>&1 < /dev/null &
sleep 2
nohup foot -T dim-b >/dev/null 2>&1 < /dev/null &
sleep 4
"$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"].startswith("dim-"):
        f = w["frame"]
        if w["title"] == "dim-a":
            # the strip of dim-a that dim-b leaves uncovered, below the titlebar
            print("CLICK", w["title"], f["x"] + 20, f["y"] + 300)'
