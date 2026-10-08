#!/usr/bin/env bash
# Guest: copy and paste works between clients, as in a GNOME session.
#
# The paths a user meets every day: wl-copy to wl-paste; a native Wayland GTK4 app setting the clipboard and wl-paste
# reading it, and the reverse; the same two directions for a GTK4 app on X11, which go through the Xwayland selection
# bridge; and the primary selection. Setting a selection needs keyboard focus, so each app is clicked before it acts.
#
# wl-clipboard 2.2.1 has no ext-data-control support, so wl-copy opens a small window and waits for focus. The test sets
# focus_new_windows = "allow" so that it gets it. With "prevent" wl-copy blocks until it is killed.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
export DISPLAY=:0 XAUTHORITY="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
F="$HOME/.config/gnoblin/config/99-test-clipboard.lua"
fail=0
trap 'rm -f "$F" /tmp/clip-result.txt; "$G" config reload >/dev/null 2>&1; pkill -x wl-copy' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' > "$F"
"$G" config reload >/dev/null 2>&1
sleep 3

cat > /tmp/clip-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import GLib, Gtk

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
    nohup env GDK_BACKEND="$1" python3 /tmp/clip-app.py "$2" "${3:-}" >/dev/null 2>&1 < /dev/null &
    sleep 2.5
    python3 "$CLICK" $(center) >/dev/null 2>&1
    sleep 8
}

stop_app() { pkill -f clip-app.py; sleep 1; }

copy one; sleep 1
check "wl-copy to wl-paste" "$(paste)" "one"

run_app wayland set two
check "a Wayland GTK app sets the clipboard and wl-paste reads it" "$(paste)" "two"
stop_app

copy three; sleep 1
rm -f /tmp/clip-result.txt
run_app wayland get
check "a Wayland GTK app reads what wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "three"
stop_app

run_app x11 set four
check "an X11 GTK app sets the clipboard and wl-paste reads it" "$(paste)" "four"
stop_app

copy five; sleep 1
rm -f /tmp/clip-result.txt
run_app x11 get
check "an X11 GTK app reads what wl-copy set" "$(cat /tmp/clip-result.txt 2>/dev/null)" "five"
stop_app

copy six --primary; sleep 1
check "the primary selection round trips" "$(paste --primary)" "six"

echo "failures: $fail"
exit "$fail"
