#!/usr/bin/env bash
# Guest: a window rule's frame.button_layout changes the buttons that Gnoblin's native frame draws.
#
# A Wayland GTK4 window started with GTK_CSD=0 negotiates server-side decorations through the KDE protocol and gets
# Gnoblin's native frame. The ink in the right end of its titlebar is counted for three layouts: the default
# (minimize, maximize, close), close only, and none (gnoblin.array {}; a plain {} is a map in Lua and does not hide
# anything). The counts must fall in that order.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-frame-buttons.lua"
fail=0

check() {
    if [ "$2" = "yes" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 ($3)"
        fail=$((fail + 1))
    fi
}

python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1

cat > /tmp/frame-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="frame-button-test")
window.set_default_size(700, 400)
window.set_child(Gtk.Label(label="frame"))
window.present()
loop = GLib.MainLoop()
GLib.timeout_add_seconds(14, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

cat > /tmp/frame-ink.py <<'PY'
import json
import subprocess
import sys
from collections import Counter

from PIL import Image

out = subprocess.run([sys.argv[1], "window", "list"], capture_output=True, text=True).stdout
frame = [w["frame"] for w in json.loads(out)["windows"] if w["title"] == "frame-button-test"][0]
im = Image.open("/tmp/frame-shot.png").convert("RGB")
x, y, w = frame["x"], frame["y"], frame["width"]
strip = [im.getpixel((px, py)) for py in range(y + 4, y + 22) for px in range(x + w - 130, x + w - 2)]
common = Counter(strip).most_common(1)[0][0]
print(sum(1 for p in strip if sum(abs(a - b) for a, b in zip(p, common)) > 60))
PY

ink_for() {
    # ink_for LAYOUT_LUA_OR_EMPTY
    if [ -n "$1" ]; then
        printf 'gnoblin.window_rule {match = {type = "window", title = "^frame%%-button%%-test$"}, frame = {button_layout = %s}}\n' "$1" > "$F"
    else
        printf -- '-- default layout\n' > "$F"
    fi
    "$G" config reload >/dev/null
    sleep 3
    nohup env GTK_CSD=0 GDK_BACKEND=wayland python3 /tmp/frame-window.py >/dev/null 2>&1 < /dev/null &
    sleep 5
    grim /tmp/frame-shot.png
    python3 /tmp/frame-ink.py "$G"
    pkill -f frame-window.py
    sleep 2
}

default_ink="$(ink_for "")"
close_ink="$(ink_for '{"close"}')"
none_ink="$(ink_for 'gnoblin.array {}')"
echo "info right-end ink: default=$default_ink close_only=$close_ink none=$none_ink"
check "close only draws less than the default three buttons" "$([ "$close_ink" -lt "$default_ink" ] && echo yes || echo no)" "$close_ink vs $default_ink"
check "an empty array draws less than close only" "$([ "$none_ink" -lt "$close_ink" ] && echo yes || echo no)" "$none_ink vs $close_ink"
check "an empty array leaves at most a few stray pixels" "$([ "$none_ink" -le 10 ] && echo yes || echo no)" "$none_ink"

rm -f "$F"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
