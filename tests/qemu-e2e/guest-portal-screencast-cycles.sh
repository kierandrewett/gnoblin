#!/usr/bin/env bash
# Guest: starting and closing a portal screen cast many times leaves nothing behind.
#
# A recorder or a browser tab that shares the screen runs CreateSession, SelectSources, Start, OpenPipeWireRemote and
# Close through the desktop portal. This repeats that flow CYCLES times (default 20) with a GStreamer consumer holding each
# stream for a moment, and checks after every Close that the compositor's PipeWire stream node is gone and the portal
# session object no longer exists. At the end it compares the open file descriptors and resident memory of the compositor
# and of the Gnoblin portal backend with the values after the first cycle, which is the warm-up. A permission rule allows
# screen-cast for the test's own python3, so no dialog appears.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-portal-screencast-cycles.lua"
CYCLES="${CYCLES:-20}"
fail=0
trap 'rm -f "$F" /tmp/portal-cycles.py /tmp/portal-cycles.jsonl; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat >"$F" <<'LUA'
gnoblin.permission_rule {
    name = "test-portal-screencast-cycles",
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
    "test-portal-screencast-cycles"

cat >/tmp/portal-cycles.py <<'PY'
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from gi.repository import Gio, GLib

CYCLES = int(sys.argv[1])
PREFIX = sys.argv[2]
HOLD = 2
DESKTOP = ("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop")
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
SENDER = bus.get_unique_name()[1:].replace(".", "_")


def pump(seconds):
    end = time.monotonic() + seconds
    ctx = GLib.MainContext.default()
    while time.monotonic() < end:
        while ctx.iteration(False):
            pass
        time.sleep(0.02)


def procs():
    """fds and VmRSS of the compositor and of the Gnoblin portal backend, keyed by role."""
    rows = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            cmd = (entry / "cmdline").read_bytes().split(b"\0")
            exe = cmd[0].decode(errors="replace") if cmd else ""
            if exe == PREFIX + "/bin/gnoblin" and b"--wayland" in cmd:
                role = "compositor"
            elif exe == PREFIX + "/libexec/xdg-desktop-portal-gnoblin":
                role = "portal-backend"
            else:
                continue
            rss = 0
            for line in (entry / "status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
            rows[role] = {"pid": int(entry.name), "fds": len(list((entry / "fd").iterdir())), "rssKiB": rss}
        except (OSError, ValueError):
            continue
    return rows


def gnoblin_nodes():
    dump = subprocess.run(["pw-dump", "--no-colors"], capture_output=True, text=True, timeout=20)
    rows = []
    for obj in json.loads(dump.stdout or "[]"):
        if "Node" not in obj.get("type", ""):
            continue
        props = (obj.get("info") or {}).get("props") or {}
        if props.get("node.name") == "gnoblin" and str(props.get("media.class", "")).startswith("Stream/Output"):
            rows.append(obj.get("id"))
    return rows


def portal(method, args, token):
    done = {}
    path = "/org/freedesktop/portal/desktop/request/%s/%s" % (SENDER, token)

    def on_response(_c, _s, _p, _i, _sig, params, _d):
        done["response"] = params.unpack()

    sub = bus.signal_subscribe(DESKTOP[0], "org.freedesktop.portal.Request", "Response", path, None,
                               Gio.DBusSignalFlags.NONE, on_response, None)
    try:
        bus.call_sync(DESKTOP[0], DESKTOP[1], "org.freedesktop.portal.ScreenCast", method, args, None,
                      Gio.DBusCallFlags.NONE, 10000, None)
        end = time.monotonic() + 25
        while "response" not in done and time.monotonic() < end:
            pump(0.05)
    finally:
        bus.signal_unsubscribe(sub)
    return done.get("response", (None, {}))


def opts(**kw):
    return {k: GLib.Variant(*v) for k, v in kw.items()}


def session_gone(path):
    try:
        bus.call_sync(DESKTOP[0], path, "org.freedesktop.portal.Session", "Close", None, None,
                      Gio.DBusCallFlags.NONE, 5000, None)
        return False
    except GLib.Error:
        return True


def cycle(index):
    tok = "c%d" % index
    session_token = "s%d" % index
    session_path = "/org/freedesktop/portal/desktop/session/%s/%s" % (SENDER, session_token)
    row = {"cycle": index, "start": None, "consumer": None, "node_after": None, "session_gone": None}
    try:
        code, results = portal("CreateSession", GLib.Variant("(a{sv})", (opts(
            handle_token=("s", tok + "a"), session_handle_token=("s", session_token)),)), tok + "a")
        session = results["session_handle"]
        portal("SelectSources", GLib.Variant("(oa{sv})", (session, opts(
            handle_token=("s", tok + "b"), types=("u", 1)))), tok + "b")
        code, results = portal("Start", GLib.Variant("(osa{sv})", (session, "", opts(
            handle_token=("s", tok + "c")))), tok + "c")
        row["start"] = code
        streams = results.get("streams", [])
        if streams:
            node = streams[0][0]
            reply, fds = bus.call_with_unix_fd_list_sync(
                DESKTOP[0], DESKTOP[1], "org.freedesktop.portal.ScreenCast", "OpenPipeWireRemote",
                GLib.Variant("(oa{sv})", (session, {})), None, Gio.DBusCallFlags.NONE, 10000, None, None)
            fd = fds.get(reply.unpack()[0])
            run = subprocess.run(
                ["timeout", str(HOLD), "gst-launch-1.0", "-q", "pipewiresrc", "fd=%d" % fd, "path=%d" % node,
                 "!", "videoconvert", "!", "fakesink", "sync=false"],
                pass_fds=[fd], capture_output=True, timeout=HOLD + 15)
            os.close(fd)
            row["consumer"] = run.returncode
    except Exception as err:  # noqa: BLE001 - record and keep cycling; Close still runs in finally
        row["error"] = repr(err)
    finally:
        session_gone(session_path)
    pump(2)
    row["node_after"] = len(gnoblin_nodes())
    row["session_gone"] = session_gone(session_path)
    row["procs"] = procs()
    return row


with open("/tmp/portal-cycles.jsonl", "w") as log:
    for index in range(1, CYCLES + 1):
        row = cycle(index)
        log.write(json.dumps(row) + "\n")
        log.flush()
PY

timeout 600 python3 /tmp/portal-cycles.py "$CYCLES" "$GNOBLIN_PREFIX" >/tmp/portal-cycles.log 2>&1
echo "python exit: $?"

summary="$(
    python3 - "$CYCLES" <<'PY'
import json
import sys

cycles = int(sys.argv[1])
rows = [json.loads(line) for line in open("/tmp/portal-cycles.jsonl")]
started = sum(1 for r in rows if r["start"] == 0)
held = sum(1 for r in rows if r["consumer"] == 124)
clean = sum(1 for r in rows if r["node_after"] == 0)
gone = sum(1 for r in rows if r["session_gone"])
first = rows[0]["procs"] if rows else {}
last = rows[-1]["procs"] if rows else {}
print("rows=%d started=%d held=%d clean=%d gone=%d" % (len(rows), started, held, clean, gone))
for role in ("compositor", "portal-backend"):
    a = first.get(role, {})
    b = last.get(role, {})
    print("%s fds=%s->%s rss=%s->%s" % (role, a.get("fds"), b.get("fds"), a.get("rssKiB"), b.get("rssKiB")))
PY
)"
echo "$summary"
field() { echo "$summary" | sed -n "s/.*$1=\([^ ]*\).*/\1/p" | head -1; }
role_field() { echo "$summary" | sed -n "s/^$1 .*$2=\([^ ]*\).*/\1/p" | head -1; }

check "every cycle starts a stream" "$(field started)" "$CYCLES"
check "the consumer holds every stream" "$(field held)" "$CYCLES"
check "the compositor's stream node is gone after every Close" "$(field clean)" "$CYCLES"
check "the portal session object is gone after every Close" "$(field gone)" "$CYCLES"
compositor_fds="$(role_field compositor fds)"
backend_fds="$(role_field portal-backend fds)"
check "the compositor keeps the same number of open files" "${compositor_fds#*->}" "${compositor_fds%->*}"
check "the portal backend keeps the same number of open files" "${backend_fds#*->}" "${backend_fds%->*}"
rss_ok() {
    # rss_ok A->B: B grew by less than 2048 KiB over A
    python3 -c 'import sys; a, b = sys.argv[1].split("->"); print("yes" if int(b) - int(a) < 2048 else "no")' "$1"
}
check "the compositor's resident memory grows by less than 2 MiB" "$(rss_ok "$(role_field compositor rss)")" "yes"
check "the portal backend's resident memory grows by less than 2 MiB" "$(rss_ok "$(role_field portal-backend rss)")" "yes"
check "the compositor still runs after the cycles" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
