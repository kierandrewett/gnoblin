#!/usr/bin/env bash
# Guest: copy and paste works between clients, as in a GNOME session.
#
# The paths a user meets every day: wl-copy to wl-paste; a native Wayland GTK4 app setting the clipboard and wl-paste
# reading it, and the reverse; the same two directions for a GTK4 app on X11, which go through the Xwayland selection
# bridge; the primary selection; and a PNG image in the same directions (an image is not text: it is converted by the
# toolkit and, for X11, by the selection bridge). Setting a selection needs keyboard focus, so each app is clicked before it acts.
#
# wl-clipboard 2.2.1 has no ext-data-control support, so wl-copy opens a small window and waits for focus. The test sets
# focus_new_windows = "allow" so that it gets it. With "prevent" wl-copy blocks until it is killed.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
XAUTHORITY="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
export DISPLAY=:0 XAUTHORITY
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
F="$HOME/.config/gnoblin/config/99-test-clipboard.lua"
fail=0
trap 'rm -f "$F" /tmp/clip-result.txt /tmp/clip-image.png; "$G" config reload >/dev/null 2>&1; pkill -x wl-copy' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$F"
"$G" config reload >/dev/null 2>&1
sleep 3

cat >/tmp/clip-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, GLib, Gtk

mode, text = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else "")
Gtk.init()
window = Gtk.Window(title="clip-app")
window.set_default_size(300, 200)
window.set_child(Gtk.Label(label="clipboard"))
window.present()
loop = GLib.MainLoop()
clipboard = window.get_clipboard()


def act():
    if mode == "set":
        clipboard.set(text)
    elif mode == "set-image":
        clipboard.set_texture(Gdk.Texture.new_from_filename("/tmp/clip-image.png"))
    elif mode == "get-image":
        def image_done(source, result):
            try:
                texture = source.read_texture_finish(result)
                value = "%sx%s" % (texture.get_width(), texture.get_height())
            except Exception as error:
                value = "ERR " + str(error)
            open("/tmp/clip-result.txt", "w").write(value)
        clipboard.read_texture_async(None, image_done)
    else:
        def done(source, result):
            try:
                value = source.read_text_finish(result)
            except Exception as error:
                value = "ERR " + str(error)
            open("/tmp/clip-result.txt", "w").write(str(value))
        clipboard.read_text_async(None, done)
    return False


# The test clicks the window at about 2.5 s. Act well after that, so the app has keyboard focus.
GLib.timeout_add(6500, act)
GLib.timeout_add_seconds(14, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "clip-app":
        f = w["frame"]; print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}

copy() {
    # copy TEXT [wl-copy options]. wl-copy forks, so its output must not be the pipe of a command substitution.
    printf %s "$1" | wl-copy "${@:2}" >/dev/null 2>&1
}

paste() { timeout 5 wl-paste -n "$@" 2>&1; }

run_app() {
    # run_app BACKEND MODE [TEXT]
    nohup env GDK_BACKEND="$1" python3 /tmp/clip-app.py "$2" "${3:-}" >/dev/null 2>&1 </dev/null &
    sleep 2.5
    read -r click_x click_y <<<"$(center)"
    python3 "$CLICK" "$click_x" "$click_y" >/dev/null 2>&1
    sleep 8
}

stop_app() {
    pkill -f clip-app.py
    sleep 1
}

copy one
sleep 1
check "wl-copy to wl-paste" "$(paste)" "one"

run_app wayland set two
check "a Wayland GTK app sets the clipboard and wl-paste reads it" "$(paste)" "two"
stop_app

copy three
sleep 1
rm -f /tmp/clip-result.txt
run_app wayland get
check "a Wayland GTK app reads what wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "three"
stop_app

run_app x11 set four
check "an X11 GTK app sets the clipboard and wl-paste reads it" "$(paste)" "four"
stop_app

copy five
sleep 1
rm -f /tmp/clip-result.txt
run_app x11 get
check "an X11 GTK app reads what wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "five"
stop_app

copy six --primary
sleep 1
check "the primary selection round trips" "$(paste --primary)" "six"

# A 16x8 PNG made with the standard library, so the test needs no imaging package.
python3 - <<'PY'
import struct
import zlib


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))


rows = b"".join(b"\x00" + bytes([40, 120, 200]) * 16 for _ in range(8))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 8, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
open("/tmp/clip-image.png", "wb").write(png)
PY

copy_image() {
    # wl-copy forks, so its output must not be the pipe of a command substitution.
    wl-copy --type image/png </tmp/clip-image.png >/dev/null 2>&1
}

png_size() {
    # Reads PNG bytes on stdin and prints WIDTHxHEIGHT, or the reason the bytes are not a PNG.
    python3 -c '
import struct
import sys

data = sys.stdin.buffer.read()
if data[:8] != b"\x89PNG\r\n\x1a\n":
    print("not a PNG (%d bytes)" % len(data))
else:
    print("%dx%d" % struct.unpack(">II", data[16:24]))
'
}

paste_image() { timeout 5 wl-paste --type image/png 2>/dev/null | png_size; }

copy_image
sleep 1
check "an image round trips from wl-copy to wl-paste byte for byte" \
    "$(timeout 5 wl-paste --type image/png 2>/dev/null | sha256sum | cut -d' ' -f1)" "$(sha256sum </tmp/clip-image.png | cut -d' ' -f1)"

run_app wayland set-image
check "a Wayland GTK app sets an image and wl-paste reads it as a PNG" "$(paste_image)" "16x8"
stop_app

copy_image
sleep 1
rm -f /tmp/clip-result.txt
run_app wayland get-image
check "a Wayland GTK app reads an image that wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "16x8"
stop_app

run_app x11 set-image
check "an X11 GTK app sets an image and wl-paste reads it as a PNG" "$(paste_image)" "16x8"
stop_app

copy_image
sleep 1
rm -f /tmp/clip-result.txt
run_app x11 get-image
check "an X11 GTK app reads an image that wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "16x8"
stop_app

echo "failures: $fail"
exit "$fail"
