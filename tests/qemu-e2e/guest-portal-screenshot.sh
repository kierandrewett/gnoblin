#!/usr/bin/env bash
# Guest: an app with no parent window gets the portal's screenshot permission dialog.
#
# The portal asks the user to allow a screenshot. The backend used to send that request to GNOME Shell when the app had no
# parent window, so the whole request failed in a Gnoblin session. The test plays such an app: it calls
# org.freedesktop.portal.Screenshot with an empty parent window. It checks that the access dialog appears with keyboard
# focus, that Deny returns no file, and that the dialog closes. Tab moves from the default Deny button to Allow, and Return
# presses the focused button. After Allow the backend still needs org.gnome.Shell.Screenshot, which a Gnoblin session does
# not provide (GitHub #142), so "Allow returns a PNG" is reported as a known gap and does not fail the run.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-portal-screenshot.lua"
DIALOG="Allow Applications to Take Screenshots?"
fail=0
trap 'pkill -f portal-shot.py 2>/dev/null; rm -f "$F" /tmp/portal-shot.py /tmp/portal-keys.py /tmp/portal-shot.out; forget_permission; "$G" config reload >/dev/null 2>&1' EXIT

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

cat >/tmp/portal-shot.py <<'PY'
import urllib.parse

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
loop = GLib.MainLoop()
result = {}


def on_response(conn, sender, path, iface, signal, params, data):
    code, results = params.unpack()
    result["code"] = code
    result["uri"] = results.get("uri")
    loop.quit()


token = "gnoblin%d" % GLib.get_monotonic_time()
sender = bus.get_unique_name()[1:].replace(".", "_")
path = "/org/freedesktop/portal/desktop/request/%s/%s" % (sender, token)
bus.signal_subscribe("org.freedesktop.portal.Desktop", "org.freedesktop.portal.Request", "Response", path, None,
                     Gio.DBusSignalFlags.NONE, on_response, None)
options = {"interactive": GLib.Variant("b", False), "handle_token": GLib.Variant("s", token)}
bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot",
              "Screenshot", GLib.Variant("(sa{sv})", ("", options)), None, Gio.DBusCallFlags.NONE, 10000, None)
GLib.timeout_add_seconds(40, lambda: (result.setdefault("code", "timeout"), loop.quit()) and False)
loop.run()
print("code=%s" % result.get("code"))
if result.get("uri"):
    print("file=%s" % urllib.parse.unquote(result["uri"][len("file://"):]))
PY

cat >/tmp/portal-keys.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

CODES = {"esc": 1, "tab": 15, "enter": 28}
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
for name in sys.argv[1].split(","):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], True)))
    time.sleep(0.1)
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], False)))
    time.sleep(0.4)
call(session, iface, "Stop", None)
PY

dialog_field() {
    # dialog_field FIELD: prints a field of the newest access dialog, or "none"
    "$G" window list | python3 -c '
import json, sys
found = [w for w in json.load(sys.stdin)["windows"] if w["title"] == "'"$DIALOG"'"]
print(found[-1]["'"$1"'"] if found else "none")'
}
wait_for_dialog() {
    for _ in $(seq 1 40); do
        if [ "$(dialog_field id)" != none ]; then return 0; fi
        sleep 0.25
    done
    return 1
}
wait_for_result() {
    for _ in $(seq 1 60); do
        if grep -q '^code=' /tmp/portal-shot.out 2>/dev/null; then return 0; fi
        sleep 0.5
    done
    return 1
}
png_size() {
    python3 -c '
import struct, sys
data = open(sys.argv[1], "rb").read()
print("%dx%d" % struct.unpack(">II", data[16:24]) if data[:8] == b"\x89PNG\r\n\x1a\n" else "not a PNG")' "$1"
}
forget_permission() {
    # The portal remembers Allow and Deny for the app and then skips the dialog. Start each request clean.
    busctl --user call org.freedesktop.impl.portal.PermissionStore /org/freedesktop/impl/portal/PermissionStore \
        org.freedesktop.impl.portal.PermissionStore DeletePermission sss screenshot screenshot "" >/dev/null 2>&1
    busctl --user call org.freedesktop.impl.portal.PermissionStore /org/freedesktop/impl/portal/PermissionStore \
        org.freedesktop.impl.portal.PermissionStore Delete ss screenshot screenshot >/dev/null 2>&1
}

ask() {
    # ask KEYS: request a screenshot, wait for the dialog, press KEYS, wait for the portal's answer
    forget_permission
    : >/tmp/portal-shot.out
    nohup python3 /tmp/portal-shot.py >/tmp/portal-shot.out 2>&1 </dev/null &
    check "the access dialog appears for an app with no parent window" "$(wait_for_dialog && echo yes)" "yes"
    check "the access dialog has keyboard focus" "$(dialog_field focused)" "True"
    python3 /tmp/portal-keys.py "$1" >/dev/null 2>&1
    wait_for_result
}

# Dialogs left by an earlier run are answered with Escape, which denies them. Then the remembered answer is cleared.
for _ in 1 2 3 4 5; do
    if [ "$(dialog_field id)" = none ]; then break; fi
    python3 /tmp/portal-keys.py esc >/dev/null 2>&1
    sleep 1
done
sleep 2

ask tab,enter
code="$(sed -n 's/^code=//p' /tmp/portal-shot.out)"
file="$(sed -n 's/^file=//p' /tmp/portal-shot.out)"
size="$(png_size "${file:-/dev/null}" 2>/dev/null)"
case "$size" in
    *x*) echo "PASS Allow returns a PNG of the screen ($size)" ;;
    *) echo "KNOWN-GAP Allow returns a PNG of the screen (GitHub #142, code: ${code:-none}, file: ${file:-none})" ;;
esac
check "the access dialog closes after the answer" "$(dialog_field id)" "none"

ask enter
check "Deny returns a response that is not success" "$([ "$(sed -n 's/^code=//p' /tmp/portal-shot.out)" != 0 ] && echo yes)" "yes"
check "Deny returns no file" "$(sed -n 's/^file=//p' /tmp/portal-shot.out)" ""

echo "failures: $fail"
exit "$fail"
