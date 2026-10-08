#!/usr/bin/env bash
# Guest: drag and drop of text between windows works, including between Wayland and X11 apps.
#
# A small GTK4 app runs as a drag source (a label that offers a text string) or a drop target (a label that records what
# it receives). A source and a target are placed side by side and the pointer drags from one to the other with a real
# left button through RemoteDesktop. The text must arrive intact for Wayland to Wayland, Wayland to X11 and X11 to
# Wayland. The X11 cases go through the Xwayland drag-and-drop bridge.
#
# Wayland to X11 does not work yet (GitHub #116). That case is reported as a known gap and does not fail the run. When
# it starts to pass the script says so, and the gap marker below should be removed.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
export DISPLAY=:0 XAUTHORITY="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
DRAG="${GNOBLIN_TITLEBAR_DRAG:-/tmp/guest-titlebar-drag.py}"
fail=0
trap 'pkill -f dnd-app.py 2>/dev/null; rm -f /tmp/dnd-result.txt' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat > /tmp/dnd-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, GLib, GObject, Gtk

role = sys.argv[1]
Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title="dnd-" + role)
window.set_default_size(300, 300)
label = Gtk.Label(label="DRAG ME" if role == "source" else "DROP HERE")
window.set_child(label)
window.present()
if role == "source":
    source = Gtk.DragSource()
    source.set_actions(Gdk.DragAction.COPY)
    source.connect("prepare", lambda s, x, y: Gdk.ContentProvider.new_for_value("dropped-text-123"))
    label.add_controller(source)
else:
    target = Gtk.DropTarget.new(GObject.TYPE_STRING, Gdk.DragAction.COPY)

    def on_drop(t, value, x, y):
        open("/tmp/dnd-result.txt", "w").write("DROP " + str(value))
        return True

    target.connect("drop", on_drop)
    label.add_controller(target)
GLib.timeout_add_seconds(30, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

id_of() {
    "$G" window list | python3 -c '
import json, sys
ids = [w["id"] for w in json.load(sys.stdin)["windows"] if w["title"] == "'"$1"'"]
print(ids[0] if ids else "")'
}
center() {
    "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "'"$1"'":
        f = w["frame"]; print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)'
}

drag_case() {
    # drag_case NAME SOURCE_BACKEND TARGET_BACKEND [KNOWN_GAP_ISSUE]
    rm -f /tmp/dnd-result.txt
    nohup env GDK_BACKEND="$2" python3 /tmp/dnd-app.py source >/dev/null 2>&1 < /dev/null &
    nohup env GDK_BACKEND="$3" python3 /tmp/dnd-app.py target >/dev/null 2>&1 < /dev/null &
    sleep 5
    "$G" window move "$(id_of dnd-source)" 100 150 >/dev/null 2>&1
    "$G" window move "$(id_of dnd-target)" 600 150 >/dev/null 2>&1
    sleep 2
    read -r sx sy <<<"$(center dnd-source)"
    read -r tx ty <<<"$(center dnd-target)"
    python3 "$DRAG" "$sx" "$sy" $((tx - sx)) $((ty - sy)) >/dev/null 2>&1
    sleep 2
    got="$(cat /tmp/dnd-result.txt 2>/dev/null)"
    if [ -n "${4:-}" ]; then
        if [ "$got" = "DROP dropped-text-123" ]; then
            echo "PASS $1 (the known gap $4 is fixed: remove the marker from this test)"
        else
            echo "KNOWN-GAP $1 ($4, got: ${got:-nothing})"
        fi
    else
        check "$1" "$got" "DROP dropped-text-123"
    fi
    pkill -f dnd-app.py
    sleep 2
}

drag_case "text dragged from a Wayland app to a Wayland app arrives intact" wayland wayland
drag_case "text dragged from a Wayland app to an X11 app arrives intact" wayland x11 "GitHub #116"
drag_case "text dragged from an X11 app to a Wayland app arrives intact" x11 wayland
check "the compositor keeps running" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
