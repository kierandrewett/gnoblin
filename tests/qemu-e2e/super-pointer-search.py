#!/usr/bin/env python3
"""Check Super tap versus pointer chords on the QEMU seat with Bingux running.

Usage: python3 tests/qemu-e2e/super-pointer-search.py RUN_DIRECTORY
Requires foot, binguxctl, qs, and gnoblinctl in the guest's installed prefixes.
Does not alter configuration or stop the shell. Saves geometry and screenshots.
"""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import sys
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
runner = Path(__file__).resolve().parents[2] / "scripts/qemu-e2e"
artifact = args.run / f"super-pointer-search-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}"
artifact.mkdir()
title = f"super-pointer-proof-{time.time_ns()}"
unit = title
env = [
    "env",
    "XDG_RUNTIME_DIR=/run/user/1000",
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
    "WAYLAND_DISPLAY=wayland-0",
    "PATH=/home/luna/.local/share/bingux/bin:"
    "/home/luna/.local/share/gnoblin/quickshell-fedora/bin:"
    "/usr/local/lib/gnoblin/bin:/usr/bin:/bin",
]


def guest(*command):
    return subprocess.check_output([str(runner), "guest", str(args.run), "--", *env, *command], timeout=20)


def ctl(*command):
    return json.loads(guest("gnoblinctl", *command))


def search():
    return json.loads(guest("binguxctl", "search", "status"))


def wait_for(predicate, message):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError(message)


def window():
    return next((w for w in ctl("window", "list")["windows"] if w["title"] == title), None)


sock = socket.socket(socket.AF_UNIX)
sock.settimeout(10)
sock.connect(str(args.run / "qmp.sock"))
stream = sock.makefile("rw")
stream.readline()


def qmp(command, **arguments):
    stream.write(json.dumps({"execute": command, "arguments": arguments}) + "\n")
    stream.flush()
    while True:
        reply = json.loads(stream.readline())
        if "error" in reply:
            raise RuntimeError(reply)
        if "return" in reply:
            return reply["return"]


qmp("qmp_capabilities")
monitors = ctl("monitor", "list")["monitors"]
assert len(monitors) == 1 and monitors[0]["x"] == 0 and monitors[0]["y"] == 0, "Use one VM monitor"
width, height = monitors[0]["width"], monitors[0]["height"]
held_keys = set()
held_buttons = set()
results = []
search_owned = False
unit_started = False


def key(code, down):
    qmp("input-send-event", events=[{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": code}}}])
    (held_keys.add if down else held_keys.discard)(code)


def button(code, down):
    qmp("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": code}}])
    (held_buttons.add if down else held_buttons.discard)(code)


def move(x, y):
    assert 0 <= x < width and 0 <= y < height
    # Force a changed tablet event even after a compositor restart.
    qmp("input-send-event", events=[{"type": "abs", "data": {"axis": "x", "value": int((x + 1) / width * 32767)}}])
    qmp(
        "input-send-event",
        events=[
            {"type": "abs", "data": {"axis": "x", "value": int(x / width * 32767)}},
            {"type": "abs", "data": {"axis": "y", "value": int(y / height * 32767)}},
        ],
    )


def capture(name):
    (artifact / f"{name}.png").write_bytes(guest("grim", "-"))


def reset():
    ctl("window", "restore", window_id)
    ctl("window", "resize", window_id, "480", "320")
    ctl("window", "move", window_id, "300", "180")
    time.sleep(0.4)
    frame = window()["frame"]
    move(frame["x"] + frame["width"] // 2, frame["y"] + frame["height"] // 2)
    button("left", True)
    button("left", False)
    wait_for(lambda: window()["focused"], "Fixture did not receive focus")
    return window()["frame"]


def drag(name, code, start, delta):
    before = window()["frame"]
    move(*start)
    time.sleep(0.1)
    key("meta_l", True)
    time.sleep(0.25)
    assert not search()["open"], "Search opened on Super press"
    button(code, True)
    time.sleep(0.15)
    for step in range(1, 13):
        move(start[0] + delta[0] * step / 12, start[1] + delta[1] * step / 12)
        time.sleep(0.04)
    button(code, False)
    time.sleep(0.1)
    key("meta_l", False)
    time.sleep(0.4)
    state = search()
    after = window()["frame"]
    results.append({"case": name, "before": before, "after": after, "search": state})
    capture(name)
    assert not state["open"], f"{name}: pointer chord opened Search on release"
    return before, after


try:
    assert not search()["open"], "Close Search before running; existing UI is left untouched"
    search_owned = True
    assert width >= 1000 and height >= 700, "VM desktop is too small for the fixture"
    # Prove that Search actually owns bare Super: an unregistered shortcut could
    # otherwise make every drag assertion pass without exercising the conflict.
    key("meta_l", True)
    time.sleep(0.25)
    assert not search()["open"], "Search opened before Super release"
    key("meta_l", False)
    wait_for(lambda: search()["open"], "Bare Super did not open Search")
    capture("bare-super-search")
    guest("binguxctl", "search", "close")
    wait_for(lambda: not search()["open"], "Search did not close")
    time.sleep(0.4)
    guest(
        "systemd-run",
        "--user",
        "--collect",
        f"--unit={unit}",
        "--setenv=WAYLAND_DISPLAY=wayland-0",
        "foot",
        "-T",
        title,
    )
    unit_started = True
    window_id = wait_for(window, "Foot fixture did not map")["id"]
    frame = reset()
    # Drag the client body, not the titlebar (which moves without Super).
    before, after = drag(
        "body-move", "left", (frame["x"] + frame["width"] / 2, frame["y"] + frame["height"] / 2), (80, 50)
    )
    assert (before["width"], before["height"]) == (after["width"], after["height"])
    assert after["x"] - before["x"] >= 60 and after["y"] - before["y"] >= 30, results[-1]
    print("PASS Super+left client-body drag moves; Search stays closed", flush=True)
    for name, fx, fy, dx, dy in (
        ("south-east", 0.75, 0.75, 80, 50),
        ("north-west", 0.25, 0.25, -80, -50),
        ("north-east", 0.75, 0.25, 80, -50),
        ("south-west", 0.25, 0.75, -80, 50),
    ):
        frame = reset()
        before, after = drag(
            name, "right", (frame["x"] + frame["width"] * fx, frame["y"] + frame["height"] * fy), (dx, dy)
        )
        assert after["width"] > before["width"] and after["height"] > before["height"], results[-1]

        # Compare the two opposite edges; tolerate client size increments.
        def opposite_x(f):
            return f["x"] if fx > 0.5 else f["x"] + f["width"]

        def opposite_y(f):
            return f["y"] if fy > 0.5 else f["y"] + f["height"]

        assert abs(opposite_x(after) - opposite_x(before)) <= 2, results[-1]
        assert abs(opposite_y(after) - opposite_y(before)) <= 2, results[-1]
        print(f"PASS Super+right {name} resize keeps opposite edges; Search stays closed", flush=True)
    frame = reset()
    drag(
        "right-edge-snap",
        "left",
        (frame["x"] + frame["width"] / 2, frame["y"] + frame["height"] / 2),
        (width - 2 - frame["x"] - frame["width"] / 2, 0),
    )
    after = window()["frame"]
    assert after["x"] > width / 3 and after["width"] < width * 0.6, results[-1]
    assert abs(after["x"] + after["width"] - width) <= 20, results[-1]
    print("PASS Super+left body drag snaps at right edge; Search stays closed", flush=True)
    key("meta_l", True)
    key("meta_l", False)
    wait_for(lambda: search()["open"], "Search stopped working after pointer chords")
    print("PASS bare Super still opens Search after all pointer chords", flush=True)
finally:
    failure_in_flight = sys.exc_info()[0] is not None
    cleanup_errors = []

    def cleanup(action):
        try:
            action()
        except Exception as error:
            cleanup_errors.append(str(error))

    for code in list(held_buttons):
        cleanup(lambda: button(code, False))
    for code in list(held_keys):
        cleanup(lambda: key(code, False))
    if unit_started:
        cleanup(lambda: guest("systemctl", "--user", "stop", unit))
    if search_owned:
        cleanup(lambda: guest("binguxctl", "search", "close"))
        cleanup(lambda: wait_for(lambda: not search()["open"], "Search remained open after cleanup"))
    (artifact / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    stream.close()
    sock.close()
    print(f"Evidence: {artifact}", flush=True)
    if cleanup_errors:
        print("Cleanup errors: " + "; ".join(cleanup_errors), flush=True)
        if not failure_in_flight:
            raise RuntimeError("Regression passed but cleanup was incomplete")
