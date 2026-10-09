#!/usr/bin/env bash
# Guest: a window rule's frame.renderer selects an external frame renderer registered in frame_renderers.
#
# A Wayland GTK4 window started with GTK_CSD=0 negotiates server-side decorations through the KDE protocol. Its titlebar
# colour is sampled with Gnoblin's native frame, then with the sample cairo renderer from src/tools/frame-renderer
# (copied to /tmp/gnoblin-frame-cairo by the host script). The cairo renderer paints a solid titlebar in the colour the
# compositor sends it, which differs from the native dark grey, so the two samples must differ.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-frame-renderer.lua"
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

cat > /tmp/renderer-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="frame-renderer-test")
window.set_default_size(700, 400)
window.set_child(Gtk.Label(label="frame"))
window.present()
loop = GLib.MainLoop()
GLib.timeout_add_seconds(14, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

# Prints the titlebar pixel as "R G B", sampled well away from the title text and the buttons.
cat > /tmp/renderer-pixel.py <<'PY'
import json
import subprocess
import sys

from PIL import Image

out = subprocess.run([sys.argv[1], "window", "list"], capture_output=True, text=True).stdout
frame = [w["frame"] for w in json.loads(out)["windows"] if w["title"] == "frame-renderer-test"][0]
im = Image.open("/tmp/renderer-shot.png").convert("RGB")
print(*im.getpixel((frame["x"] + 300, frame["y"] + 16)))
PY

titlebar_pixel() {
    nohup env GTK_CSD=0 GDK_BACKEND=wayland python3 /tmp/renderer-window.py >/dev/null 2>&1 < /dev/null &
    sleep 6
    grim /tmp/renderer-shot.png
    python3 /tmp/renderer-pixel.py "$G"
    pkill -f renderer-window.py
    sleep 2
}

printf -- '-- native frame\n' > "$F"
"$G" config reload >/dev/null 2>&1
sleep 3
native_pixel="$(titlebar_pixel)"

cat > "$F" <<'LUA'
gnoblin.configure {frame_renderers = {cairo = {"/tmp/gnoblin-frame-cairo"}}}
gnoblin.window_rule {match = {type = "window", title = "^frame%-renderer%-test$"},
                     frame = {mode = "auto", renderer = "cairo"}}
LUA
if "$G" config reload >/dev/null 2>&1; then
    check "a config with an external frame renderer reloads" yes ""
else
    check "a config with an external frame renderer reloads" no "reload failed"
fi
sleep 3
cairo_pixel="$(titlebar_pixel)"

echo "info titlebar pixel: native=($native_pixel) cairo=($cairo_pixel)"
check "the external renderer paints a different titlebar than the native frame" \
    "$([ "$native_pixel" != "$cairo_pixel" ] && [ -n "$cairo_pixel" ] && echo yes || echo no)" "$native_pixel vs $cairo_pixel"
set -- $cairo_pixel
check "the titlebar carries the colour the compositor sends the renderer" \
    "$(python3 -c "import sys; r,g,b=map(int,sys.argv[1:4]); print('yes' if max(abs(r-32),abs(g-64),abs(b-128))<=12 else 'no')" "$@" 2>/dev/null || echo no)" "$cairo_pixel"

rm -f "$F"
"$G" config reload >/dev/null 2>&1
echo "failures: $fail"
exit "$fail"
