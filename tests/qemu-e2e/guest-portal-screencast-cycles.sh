#!/usr/bin/env bash
# Guest: starting and closing a portal screen cast many times leaves nothing behind, and every cycle delivers frames.
#
# A recorder or a browser tab that shares the screen runs CreateSession, SelectSources, Start and OpenPipeWireRemote through
# org.freedesktop.portal.Desktop, and then Session.Close. The consent dialog ("Share Screen") appears on every Start. The test
# opens it, waits until it has keyboard focus, and confirms it with Tab, Tab, Return, which moves focus past Cancel to Share
# (Return alone does nothing, Tab then Return cancels). Each cycle checks that Start returns code 0 with a PipeWire node, that
# GStreamer's pipewiresrc reads five frames from that node, that the compositor's stream node exists while the session runs,
# and that the node and the portal session object are gone after Close. Every SAMPLE_EVERY cycles and at the end, the test
# records the open file descriptors and resident memory of the compositor, the Gnoblin portal backend and PipeWire. Growth is
# measured from the first cycle, which is the warm-up. CYCLES (default 10) sets how many cycles run.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-portal-screencast-cycles.lua"
CYCLES="${CYCLES:-10}"
fail=0
trap 'rm -f "$F" /tmp/portal-cycles.py /tmp/portal-cycles.jsonl /tmp/portal-cycles.log; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

# The dialog must get focus so that the keys reach it. This is the same setting the portal screenshot test uses.
printf 'gnoblin.configure {window_management = {focus_new_windows = "allow"}}\n' >"$F"
"$G" config reload >/dev/null 2>&1
sleep 3

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
GNOBLINCTL = PREFIX + "/bin/gnoblinctl"
DIALOG = "Share Screen"
DESKTOP = "org.freedesktop.portal.Desktop"
DESKTOP_PATH = "/org/freedesktop/portal/desktop"
REMOTE = "org.gnome.Mutter.RemoteDesktop"
KEY_TAB, KEY_RETURN = 15, 28
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
SENDER = bus.get_unique_name()[1:].replace(".", "_")
ctx = GLib.MainContext.default()


def pump(seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while ctx.iteration(False):
            pass
        time.sleep(0.02)


def procs():
    """fds and VmRSS of the compositor, the Gnoblin portal backend and PipeWire, keyed by role."""
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
            elif Path(exe).name == "pipewire" and "pipewire" not in rows:
                role = "pipewire"
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


def stream_nodes():
    dump = subprocess.run(["pw-dump", "--no-colors"], capture_output=True, text=True, timeout=20)
    nodes = []
    for obj in json.loads(dump.stdout or "[]"):
        if "Node" not in obj.get("type", ""):
            continue
        props = (obj.get("info") or {}).get("props") or {}
        if props.get("node.name") == "gnoblin" and str(props.get("media.class", "")).startswith("Stream/Output"):
            nodes.append((obj.get("id"), props.get("object.serial"), (obj.get("info") or {}).get("state"), props.get("media.class"), props.get("pipewire.access")))
    return nodes


def dialog_focused():
    out = subprocess.run([GNOBLINCTL, "window", "list", "--json"], capture_output=True, text=True).stdout
    windows = [w for w in json.loads(out or "{}").get("windows", []) if w.get("title") == DIALOG]
    return bool(windows) and windows[-1].get("focused") is True


def key_presses(codes):
    """Sends key presses through a RemoteDesktop session, the way a person at the keyboard would."""
    session = bus.call_sync(REMOTE, "/org/gnome/Mutter/RemoteDesktop", REMOTE, "CreateSession", None, None,
                            Gio.DBusCallFlags.NONE, 10000, None).unpack()[0]
    kbd = "org.gnome.Mutter.RemoteDesktop.Session"
    bus.call_sync(REMOTE, session, kbd, "Start", None, None, Gio.DBusCallFlags.NONE, 10000, None)
    pump(0.5)
    for code in codes:
        bus.call_sync(REMOTE, session, kbd, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, True)), None,
                      Gio.DBusCallFlags.NONE, 10000, None)
        pump(0.1)
        bus.call_sync(REMOTE, session, kbd, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, False)), None,
                      Gio.DBusCallFlags.NONE, 10000, None)
        pump(0.4)
    bus.call_sync(REMOTE, session, kbd, "Stop", None, None, Gio.DBusCallFlags.NONE, 10000, None)


def request(method, sig, args, token):
    """Calls a portal method that answers with a Request, and returns (response code, results) once the answer arrives."""
    done = {}
    path = "%s/request/%s/%s" % (DESKTOP_PATH, SENDER, token)

    def on_response(_c, _s, _p, _i, _sig, params, _d):
        done["response"] = params.unpack()

    sub = bus.signal_subscribe(DESKTOP, "org.freedesktop.portal.Request", "Response", path, None,
                               Gio.DBusSignalFlags.NONE, on_response, None)
    try:
        bus.call_sync(DESKTOP, DESKTOP_PATH, "org.freedesktop.portal.ScreenCast", method, GLib.Variant(sig, args), None,
                      Gio.DBusCallFlags.NONE, 10000, None)
        if method == "Start":
            end = time.monotonic() + 15
            while not dialog_focused() and time.monotonic() < end:
                pump(0.25)
            if dialog_focused():
                key_presses([KEY_TAB, KEY_TAB, KEY_RETURN])
            else:
                done["dialog"] = "never focused"
        end = time.monotonic() + 40
        while "response" not in done and time.monotonic() < end:
            pump(0.05)
    finally:
        bus.signal_unsubscribe(sub)
    if "response" not in done:
        return None, {}
    return done["response"]


def opts(**kw):
    return {k: GLib.Variant(*v) for k, v in kw.items()}


def session_gone(path):
    try:
        bus.call_sync(DESKTOP, path, "org.freedesktop.portal.Session", "Close", None, None,
                      Gio.DBusCallFlags.NONE, 5000, None)
        return False
    except GLib.Error:
        return True


def read_frames(node):
    """GStreamer reads five frames from the stream's PipeWire node by id, as guest-screencast.sh does. WirePlumber can refuse a
    consumer that connects before it has registered the new node ("target not found"), so a failed read retries with backoff.
    Returns (exit code, last warning)."""
    rc, last = None, ""
    for _ in range(8):
        run = subprocess.run(
            ["timeout", "30", "gst-launch-1.0", "-q", "pipewiresrc", "path=%s" % node, "num-buffers=5", "!",
             "videoconvert", "!", "fakesink"],
            capture_output=True, text=True, timeout=45, env=dict(os.environ, GST_DEBUG="3"))
        rc = run.returncode
        lines = [l for l in run.stderr.splitlines() if "ERROR" in l or "WARN" in l]
        last = " | ".join(lines[-2:])
        if rc == 0:
            break
        pump(0.5)
    return rc, last


def cycle(index):
    tok = "c%d" % index
    session_token = "s%d" % index
    row = {"cycle": index, "start": None, "node": None, "frames": None, "node_during": None, "node_after": None,
           "session_gone": None, "error": None}
    session_path = None
    try:
        code, results = request("CreateSession", "(a{sv})", (opts(
            handle_token=("s", tok + "a"), session_handle_token=("s", session_token)),), tok + "a")
        if code != 0:
            raise RuntimeError("CreateSession returned %s" % code)
        session_path = results["session_handle"]
        code, _ = request("SelectSources", "(oa{sv})", (session_path, opts(
            handle_token=("s", tok + "b"), types=("u", 1))), tok + "b")
        if code != 0:
            raise RuntimeError("SelectSources returned %s" % code)
        code, results = request("Start", "(osa{sv})", (session_path, "", opts(
            handle_token=("s", tok + "c"))), tok + "c")
        row["start"] = code
        streams = results.get("streams", [])
        if code != 0 or not streams:
            raise RuntimeError("Start returned %s with %d streams" % (code, len(streams)))
        node = streams[0][0]
        row["node"] = node
        reply, fds = bus.call_with_unix_fd_list_sync(
            DESKTOP, DESKTOP_PATH, "org.freedesktop.portal.ScreenCast", "OpenPipeWireRemote",
            GLib.Variant("(oa{sv})", (session_path, {})), None, Gio.DBusCallFlags.NONE, 10000, None, None)
        fd = fds.get(reply.unpack()[0])
        pump(1)
        during = stream_nodes()
        row["node_during"] = len(during)
        row["serial"] = during[0][1] if during else None
        row["frames"], row["gst"] = read_frames(row["node"])
        os.close(fd)
    except Exception as err:  # noqa: BLE001 - record the failure and keep cycling
        row["error"] = repr(err)
    finally:
        if session_path:
            session_gone(session_path)
    pump(2)
    row["node_after"] = len(stream_nodes())
    row["session_gone"] = session_path is not None and session_gone(session_path)
    row["procs"] = procs()
    return row


with open("/tmp/portal-cycles.jsonl", "w") as log:
    for index in range(1, CYCLES + 1):
        row = cycle(index)
        log.write(json.dumps(row) + "\n")
        log.flush()
        print(json.dumps({k: v for k, v in row.items() if k != "procs"}), flush=True)
PY

timeout 1200 python3 /tmp/portal-cycles.py "$CYCLES" "$GNOBLIN_PREFIX" >/tmp/portal-cycles.log 2>&1
echo "python exit: $?"
cat /tmp/portal-cycles.log

summary="$(
    python3 - "$CYCLES" <<'PY'
import json
import sys

cycles = int(sys.argv[1])
rows = [json.loads(line) for line in open("/tmp/portal-cycles.jsonl")]
started = sum(1 for r in rows if r["start"] == 0)
streamed = sum(1 for r in rows if r["frames"] == 0)
during = sum(1 for r in rows if r["node_during"] == 1)
clean = sum(1 for r in rows if r["node_after"] == 0)
gone = sum(1 for r in rows if r["session_gone"])
errors = sum(1 for r in rows if r["error"])
first = rows[0]["procs"] if rows else {}
last = rows[-1]["procs"] if rows else {}
print("rows=%d started=%d streamed=%d node_during=%d clean=%d gone=%d errors=%d" % (
    len(rows), started, streamed, during, clean, gone, errors))
for role in ("compositor", "portal-backend", "pipewire"):
    a = first.get(role, {})
    b = last.get(role, {})
    print("%s fds=%s->%s rss=%s->%s" % (role, a.get("fds"), b.get("fds"), a.get("rssKiB"), b.get("rssKiB")))
PY
)"
echo "$summary"
field() { echo "$summary" | sed -n "s/.*$1=\([^ ]*\).*/\1/p" | head -1; }
role_field() { echo "$summary" | sed -n "s/^$1 .*$2=\([^ ]*\).*/\1/p" | head -1; }

check "every cycle completes without an error" "$(field errors)" "0"
check "every cycle starts a stream" "$(field started)" "$CYCLES"
check "every cycle reads five frames from the stream" "$(field streamed)" "$CYCLES"
check "the compositor's stream node exists while each session runs" "$(field node_during)" "$CYCLES"
check "the compositor's stream node is gone after every Close" "$(field clean)" "$CYCLES"
check "the portal session object is gone after every Close" "$(field gone)" "$CYCLES"
for role in compositor portal-backend pipewire; do
    fds="$(role_field $role fds)"
    check "$role keeps the same number of open files" "${fds#*->}" "${fds%->*}"
done
rss_ok() {
    # rss_ok A->B: B grew by less than 2048 KiB over A
    python3 -c 'import sys; a, b = sys.argv[1].split("->"); print("yes" if int(b) - int(a) < 2048 else "no")' "$1"
}
for role in compositor portal-backend pipewire; do
    check "$role resident memory grows by less than 2 MiB" "$(rss_ok "$(role_field $role rss)")" "yes"
done
check "the compositor still runs after the cycles" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
