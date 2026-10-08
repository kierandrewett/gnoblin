#!/usr/bin/env bash
# Guest: screen sharing through the desktop portal works end to end, as in a browser or a recorder.
#
# A permission rule allows screen-cast for the test's own python3, so no dialog appears. The test runs the portal flow an app
# runs: CreateSession, SelectSources, Start, then OpenPipeWireRemote. It reads one frame from the stream with GStreamer
# through the file descriptor the portal returns, and checks that the frame is a monitor-sized PNG showing a window and not
# only the black desktop. It also checks that Gnoblin's permission policy reports the rule it matched. The dialog that
# shows when no rule matches is not covered.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-portal-screencast.lua"
fail=0
trap 'pkill -f cast-window.py 2>/dev/null; rm -f "$F" /tmp/cast-window.py /tmp/portal-cast.py /tmp/portal-cast.out /tmp/portal-cast.png; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1

cat >"$F" <<'LUA'
gnoblin.configure {window_management = {focus_new_windows = "allow"}}
gnoblin.permission_rule {
    name = "test-portal-screencast",
    match = "^host%-exe:/usr/bin/python3",
    capabilities = {"screen-cast"},
    level = "allow",
    monitors = {"primary"},
}
LUA
"$G" config reload >/dev/null 2>&1
sleep 3

check "the permission policy reports the allow rule for the test's python3" \
    "$("$G" permissions check screen-cast host-exe:/usr/bin/python3 --json 2>&1 | python3 -c 'import json,sys; print(json.load(sys.stdin).get("rule"))')" \
    "test-portal-screencast"

cat >/tmp/cast-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title="cast-window")
window.set_default_size(500, 400)
window.set_child(Gtk.Label(label="portal cast test"))
window.present()
GLib.timeout_add_seconds(90, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

cat >/tmp/portal-cast.py <<'PY'
import subprocess

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
sender = bus.get_unique_name()[1:].replace(".", "_")
DESKTOP = ("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop")


def portal(method, args, token):
    done = {}
    inner = GLib.MainLoop()

    def on_response(conn, snd, path, iface, signal, params, data):
        done["response"] = params.unpack()
        inner.quit()

    bus.signal_subscribe(DESKTOP[0], "org.freedesktop.portal.Request", "Response",
                         "/org/freedesktop/portal/desktop/request/%s/%s" % (sender, token), None,
                         Gio.DBusSignalFlags.NONE, on_response, None)
    bus.call_sync(DESKTOP[0], DESKTOP[1], "org.freedesktop.portal.ScreenCast", method, args, None,
                  Gio.DBusCallFlags.NONE, 10000, None)
    GLib.timeout_add_seconds(25, lambda: (done.setdefault("response", (None, {})), inner.quit()) and False)
    inner.run()
    return done["response"]


def opts(**kw):
    return {k: GLib.Variant(*v) for k, v in kw.items()}


code, results = portal("CreateSession", GLib.Variant("(a{sv})", (opts(handle_token=("s", "c1"), session_handle_token=("s", "s1")),)), "c1")
print("create=%s" % code, flush=True)
session = results["session_handle"]
code, _ = portal("SelectSources", GLib.Variant("(oa{sv})", (session, opts(handle_token=("s", "c2"), types=("u", 1)))), "c2")
print("select=%s" % code, flush=True)
code, results = portal("Start", GLib.Variant("(osa{sv})", (session, "", opts(handle_token=("s", "c3")))), "c3")
streams = results.get("streams", [])
print("start=%s streams=%d" % (code, len(streams)), flush=True)
if streams:
    node = streams[0][0]
    reply, fds = bus.call_with_unix_fd_list_sync(
        DESKTOP[0], DESKTOP[1], "org.freedesktop.portal.ScreenCast", "OpenPipeWireRemote",
        GLib.Variant("(oa{sv})", (session, {})), None, Gio.DBusCallFlags.NONE, 10000, None, None)
    fd = fds.get(reply.unpack()[0])
    run = subprocess.run(
        ["gst-launch-1.0", "-q", "pipewiresrc", "fd=%d" % fd, "path=%d" % node, "num-buffers=15", "!", "videoconvert",
         "!", "pngenc", "snapshot=true", "!", "filesink", "location=/tmp/portal-cast.png"],
        pass_fds=[fd], timeout=30, capture_output=True, text=True)
    print("gst=%d" % run.returncode, flush=True)
    bus.call_sync(DESKTOP[0], "/org/freedesktop/portal/desktop/session/%s/s1" % sender, "org.freedesktop.portal.Session",
                  "Close", None, None, Gio.DBusCallFlags.NONE, 5000, None)
PY

nohup env GDK_BACKEND=wayland python3 /tmp/cast-window.py >/dev/null 2>&1 </dev/null &
sleep 3
timeout 90 python3 /tmp/portal-cast.py >/tmp/portal-cast.out 2>&1
field() { sed -n "s/.*$1=\([^ ]*\).*/\1/p" /tmp/portal-cast.out | head -1; }

check "CreateSession succeeds" "$(field create)" "0"
check "SelectSources succeeds" "$(field select)" "0"
check "Start succeeds with no dialog and returns a stream" "$(sed -n 's/^start=\([0-9]*\) streams=\([0-9]*\)$/\1 \2/p' /tmp/portal-cast.out)" "0 1"
check "GStreamer reads a frame through the portal's PipeWire connection" "$(field gst)" "0"

monitor="$("$G" monitor list --json | python3 -c 'import json,sys; m=[m for m in json.load(sys.stdin)["monitors"] if m["primary"]][0]; print(int(m["width"]*m["scale"]), int(m["height"]*m["scale"]))')"
result="$(
    python3 - <<'PY' 2>&1
from PIL import Image

image = Image.open("/tmp/portal-cast.png").convert("RGB")
pixels = image.getdata()
lit = sum(1 for p in pixels if max(p) > 24)
print("%dx%d %s" % (image.width, image.height, "yes" if lit / len(pixels) > 0.02 else "no"))
PY
)"
check "the frame has the size of the primary monitor" "${result%% *}" "$(echo "$monitor" | tr ' ' 'x')"
check "the frame shows a window and not only the black desktop" "${result##* }" "yes"
check "the compositor still runs after the portal session closes" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
