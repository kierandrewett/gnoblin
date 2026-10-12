#!/usr/bin/env python3
"""Exercise installed Bingux dock minimise and restore in a running QEMU guest.

Usage: python3 tests/qemu-e2e/bingux-dock-animations.py RUN_DIRECTORY

The guest must already be booted into an installed Bingux/Gnoblin session with
foot, binguxctl and gnoblinctl available.  This runner neither starts nor stops
the VM.  It owns one Foot window and one short-lived public compositor-event
listener; both are removed on exit.  Screenshots and JSON evidence are saved
under RUN_DIRECTORY.
"""

import argparse
import base64
import json
from pathlib import Path
import socket
import subprocess
import sys
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
run = args.run.resolve()
runner = Path(__file__).resolve().parents[2] / "scripts/qemu-e2e"
artifact = run / f"bingux-dock-animations-{time.time_ns()}"
artifact.mkdir()
title = f"gnoblin-dock-animation-{time.time_ns()}"
unit = f"gnoblin-dock-animation-{time.time_ns()}"
event_unit = unit + "-events"
event_file = f"/tmp/{event_unit}.jsonl"
env = [
    "env",
    "XDG_RUNTIME_DIR=/run/user/1000",
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
    "WAYLAND_DISPLAY=wayland-0",
    "PATH=/home/luna/.local/share/bingux/bin:"
    "/home/luna/.local/share/gnoblin/quickshell-fedora/bin:"
    "/usr/local/lib/gnoblin/bin:/usr/bin:/bin",
]
results = {"title": title, "events": []}
owned_window = False
owned_listener = False
sock = None
stream = None


def guest(*command, timeout=20):
    return subprocess.check_output([str(runner), "guest", str(run), "--", *env, *command], timeout=timeout)


def json_guest(*command):
    return json.loads(guest(*command))


def ctl(*command):
    return json_guest("gnoblinctl", *command)


def windows():
    value = ctl("window", "list")
    return value if isinstance(value, list) else value.get("windows", [])


def window():
    return next((item for item in windows() if item.get("title") == title), None)


def wait_for(predicate, message, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError(message)


def qmp(command, **arguments):
    stream.write(json.dumps({"execute": command, "arguments": arguments}) + "\n")
    stream.flush()
    while True:
        reply = json.loads(stream.readline())
        if "error" in reply:
            raise RuntimeError(reply)
        if "return" in reply:
            return reply["return"]


def move(x, y):
    assert 0 <= x < desktop_width and 0 <= y < desktop_height, (x, y, desktop_width, desktop_height)
    # Flush the tablet's first absolute motion separately, as in the Super
    # pointer fixture; batching the nudge can leave the seat at its old point.
    qmp(
        "input-send-event",
        events=[{"type": "abs", "data": {"axis": "x", "value": int((x + 1) / desktop_width * 32767)}}],
    )
    qmp(
        "input-send-event",
        events=[
            {"type": "abs", "data": {"axis": "x", "value": int(x / desktop_width * 32767)}},
            {"type": "abs", "data": {"axis": "y", "value": int(y / desktop_height * 32767)}},
        ],
    )


def click(x, y, transition=None):
    move(x, y)
    time.sleep(0.12)
    qmp("input-send-event", events=[{"type": "btn", "data": {"down": True, "button": "left"}}])
    time.sleep(0.08)
    qmp("input-send-event", events=[{"type": "btn", "data": {"down": False, "button": "left"}}])
    if transition:
        started = time.monotonic()
        frames = []
        for index in range(20):
            frames.append(
                {"seconds_after_release": time.monotonic() - started, "frame": capture(f"{transition}-{index:02d}")}
            )
            time.sleep(0.02)
        results[transition + "_frames"] = frames


def capture(label):
    path = artifact / f"{label}.ppm"
    qmp("screendump", filename=str(path))
    assert path.exists() and path.stat().st_size > 64, f"QMP did not write {path}"
    return path.name


def guest_python(source, *arguments, timeout=20):
    encoded = base64.b64encode(source.encode()).decode()
    return guest(
        "python3",
        "-c",
        "import base64,sys;exec(compile(base64.b64decode(sys.argv[1]), '<guest>', 'exec'))",
        encoded,
        *arguments,
        timeout=timeout,
    )


# Read a resolved animation through the supported native socket API.  The
# targetGeom field reports the same icon rectangle used by the lifecycle path.
INSPECT = r"""
import json, os, socket, sys
path = os.environ['XDG_RUNTIME_DIR'] + '/gnoblin/compositor-v1.sock'
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(5); sock.connect(path)
stream = sock.makefile('rwb', buffering=0)
hello = json.loads(stream.readline())
minor = hello['api_minor']
request = {'op':'api', 'api_version':{'major':1, 'minor':minor}, 'id':'inspect',
           'method':'animation.inspect',
           'arguments':{'name':sys.argv[2], 'target':sys.argv[3], 'event':sys.argv[4], 'target_type':'window'}}
stream.write(json.dumps(request).encode() + b'\n')
while True:
    reply = json.loads(stream.readline())
    if reply.get('id') == 'inspect':
        if 'error' in reply: raise SystemExit(reply['error'])
        print(json.dumps(reply['result']))
        break
"""


# A temporary guest process listens only to public animation events.  Its file
# is a bounded local artifact and is deleted during cleanup.
LISTENER = r"""
import json, os, select, socket, sys, time
path, output, target = sys.argv[2:]
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.connect(path)
buffer = b''
def receive(deadline):
    global buffer
    while b'\n' not in buffer:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('timed out waiting for compositor socket data')
        readable, _, _ = select.select([sock], [], [], remaining)
        if not readable:
            raise TimeoutError('timed out waiting for compositor socket data')
        chunk = sock.recv(4096)
        if not chunk:
            raise EOFError('compositor socket closed')
        buffer += chunk
    line, buffer = buffer.split(b'\n', 1)
    return json.loads(line)
hello = receive(time.monotonic() + 5); minor = hello['api_minor']
events = ['gnoblin.animation.started', 'gnoblin.animation.finished']
sock.sendall(json.dumps({'op':'events', 'api_version':{'major':1, 'minor':minor}, 'events':events}).encode()+b'\n')
assert receive(time.monotonic() + 5).get('event') == 'subscribed'
deadline = time.monotonic() + 12
with open(output, 'w', encoding='utf-8') as log:
    log.write('{"ready":true}\n'); log.flush()
    while time.monotonic() < deadline:
        try: event = receive(min(deadline, time.monotonic() + 1))
        except TimeoutError: continue
        if event.get('event') in events and event.get('target') == target:
            log.write(json.dumps(event, separators=(',', ':')) + '\n'); log.flush()
"""


def inspect(name, event, window_id):
    return json.loads(guest_python(INSPECT, name, window_id, event))


def event_records():
    try:
        data = guest("cat", event_file).decode()
    except subprocess.CalledProcessError:
        return []
    return [json.loads(line) for line in data.splitlines() if line]


def event_listener_ready():
    try:
        return '"ready":true' in guest("cat", event_file).decode()
    except subprocess.CalledProcessError:
        return False


def target_geometry(inspection):
    assert isinstance(inspection, dict), inspection
    context = inspection.get("context")
    assert isinstance(context, dict), inspection
    target = context.get("targetGeom")
    assert isinstance(target, list) and len(target) == 2 and target[0] is True and isinstance(target[1], dict), (
        inspection
    )
    rect = target[1]
    assert rect["width"] > 0 and rect["height"] > 0, inspection
    return rect


def published_target(window_id):
    try:
        return target_geometry(inspect("gnome-minimize", "minimize", window_id))
    except (AssertionError, subprocess.CalledProcessError, json.JSONDecodeError):
        return None


def expect_events(expected, timeout=8):
    def matching_events():
        records = event_records()
        actual = [(event.get("event"), event.get("animation"), event.get("animation_event")) for event in records]
        cursor = 0
        matched = []
        for item in expected:
            cursor = next((index + 1 for index in range(cursor, len(actual)) if actual[index] == item), None)
            if cursor is None:
                return None
            matched.append(records[cursor - 1])
        return matched

    return wait_for(matching_events, f"missing animation events: {expected}", timeout)


try:
    monitors = ctl("monitor", "list").get("monitors", [])
    assert len(monitors) == 1 and monitors[0]["x"] == 0 and monitors[0]["y"] == 0, monitors
    desktop_width, desktop_height = monitors[0]["width"], monitors[0]["height"]
    assert desktop_width >= 1000 and desktop_height >= 700, monitors
    assert not window(), f"fixture title already exists: {title}"
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(10)
    sock.connect(str(run / "qmp.sock"))
    stream = sock.makefile("rw")
    stream.readline()
    qmp("qmp_capabilities")

    guest("systemd-run", "--user", "--collect", f"--unit={unit}", "foot", "-T", title)
    owned_window = True
    initial = wait_for(window, "Foot fixture did not map")
    results["initial"] = initial
    capture("initial-mapped")
    window_id = initial["id"]
    frame = initial["frame"]
    click(frame["x"] + frame["width"] // 2, frame["y"] + frame["height"] // 2)
    wait_for(
        lambda: window() and window().get("focused") is True, "Foot did not receive focus from a real client click"
    )
    # Bingux may discover a new toplevel after its first layout pass.
    minimize_inspection = inspect("gnome-minimize", "minimize", window_id)
    target = wait_for(lambda: published_target(window_id), "Bingux did not publish a dock icon rectangle")
    target_center = (target["x"] + target["width"] // 2, target["y"] + target["height"] // 2)
    assert 0 <= target_center[0] < desktop_width and 0 <= target_center[1] < desktop_height, target
    results.update(
        window_id=window_id,
        initial=initial,
        target=target,
        target_center=target_center,
        minimize_inspection=minimize_inspection,
    )
    capture("before-dock-click")

    listener_path = "/run/user/1000/gnoblin/compositor-v1.sock"
    guest("rm", "-f", event_file)
    guest(
        "systemd-run",
        "--user",
        "--collect",
        f"--unit={event_unit}",
        "/usr/bin/python3",
        "-c",
        "import base64,sys;exec(compile(base64.b64decode(sys.argv[1]), '<event-listener>', 'exec'))",
        base64.b64encode(LISTENER.encode()).decode(),
        listener_path,
        event_file,
        window_id,
    )
    owned_listener = True
    wait_for(event_listener_ready, "animation event listener did not subscribe")

    click(*target_center, transition="minimize")
    minimized = wait_for(
        lambda: window() if window() and window()["minimized"] else None, "dock click did not minimize Foot"
    )
    minimize_events = expect_events(
        [
            ("gnoblin.animation.started", "gnome-minimize", "minimize"),
            ("gnoblin.animation.finished", "gnome-minimize", "minimize"),
        ]
    )
    capture("minimized")
    click(*target_center, transition="restore")
    restored = wait_for(
        lambda: window() if window() and not window()["minimized"] and window()["focused"] else None,
        "second dock click did not restore and focus Foot",
    )
    lifecycle_events = expect_events(
        [
            ("gnoblin.animation.started", "gnome-minimize", "minimize"),
            ("gnoblin.animation.finished", "gnome-minimize", "minimize"),
            ("gnoblin.animation.started", "gnome-restore", "restore"),
            ("gnoblin.animation.finished", "gnome-restore", "restore"),
        ]
    )
    capture("restored-focused")
    events = event_records()
    results.update(
        minimized=minimized,
        restored=restored,
        events=events,
        minimize_events=minimize_events,
        lifecycle_events=lifecycle_events,
    )
    assert all(event.get("cancelled") is False for event in lifecycle_events), lifecycle_events
    print("PASS installed Bingux dock click minimized then restored/focused Foot through its published icon target")
    print("PASS public compositor events resolved gnome-minimize then gnome-restore without cancellation")
finally:
    failed = sys.exc_info()[0] is not None
    cleanup_errors = []
    for command in (
        ("systemctl", "--user", "stop", event_unit + ".service"),
        ("rm", "-f", event_file),
        ("systemctl", "--user", "stop", unit + ".service"),
    ):
        try:
            if command[-1] == unit + ".service" and not owned_window:
                continue
            if command[-1] == event_unit + ".service" and not owned_listener:
                continue
            guest(*command)
        except Exception as error:
            cleanup_errors.append(str(error))
    results["cleanup_errors"] = cleanup_errors
    (artifact / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    if stream is not None:
        stream.close()
    if sock is not None:
        sock.close()
    print(f"Evidence: {artifact}")
    if cleanup_errors and not failed:
        raise RuntimeError("cleanup failed: " + "; ".join(cleanup_errors))
