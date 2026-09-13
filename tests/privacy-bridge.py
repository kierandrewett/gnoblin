#!/usr/bin/env python3
"""Real Mutter recording and sharing sessions on a private bus."""

import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402 - Select GI versions before importing their modules.

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET", "").startswith("/tmp/")
repo = Path(__file__).resolve().parents[1]
scripts = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/scripts"
scripts.mkdir(parents=True, exist_ok=True)
shutil.copy2(repo / "src/scripts/compositor-bridge.js", scripts)
subprocess.run([str(repo / "src/tools/gnoblinctl"), "script", "reload"], check=True)
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
name = "org.gnome.Mutter.ScreenCast"


def call(path, interface, method, signature=None, args=()):
    result = bus.call_sync(
        name,
        path,
        interface,
        method,
        GLib.Variant(signature, args) if signature else None,
        None,
        Gio.DBusCallFlags.NONE,
        3000,
        None,
    )
    return result.unpack()


class Subscriber:
    def __init__(self):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(3)
        self.socket.connect(os.environ["GNOBLIN_COMPOSITOR_SOCKET"])
        self.file = self.socket.makefile("rwb", buffering=0)
        self.file.readline()
        self.send("privacy")

    def send(self, op):
        self.file.write((json.dumps({"op": op}) + "\n").encode())

    def wait(self, predicate):
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            record = json.loads(self.file.readline())
            if record["event"] == "privacy" and predicate(record):
                return record
        raise AssertionError("Expected privacy state did not arrive")

    def close(self):
        self.file.close()
        self.socket.close()


def start(recording):
    path = call("/org/gnome/Mutter/ScreenCast", name, "CreateSession", "(a{sv})", ({},))[0]
    props = {"cursor-mode": GLib.Variant("u", 0), "is-recording": GLib.Variant("b", recording)}
    call(path, name + ".Session", "RecordMonitor", "(sa{sv})", ("", props))
    call(path, name + ".Session", "Start")
    return path


client = Subscriber()
sessions = []
try:
    initial = client.wait(lambda state: not state["recording"] and not state["screenSharing"])
    assert isinstance(initial.get("locationCaptures"), list), "Privacy snapshots include location captures"
    sessions.append(start(True))
    client.wait(lambda state: state["recording"] and state["recordingCount"] == 1)
    time.sleep(1.1)
    client.send("privacy")
    before = client.wait(lambda state: state["recordingElapsed"] >= 1)
    sessions.append(start(False))
    client.wait(lambda state: state["recording"] and state["screenSharing"])
    subprocess.run([str(repo / "src/tools/gnoblinctl"), "script", "reload"], check=True)
    client.close()
    client = Subscriber()
    after = client.wait(lambda state: state["recording"] and state["screenSharing"])
    assert after["recordingElapsed"] >= before["recordingElapsed"], "Reload reset the timer"
    client.send("stop-sharing")
    client.wait(lambda state: not state["screenSharing"] and state["recording"])
    client.send("stop-recording")
    client.wait(lambda state: not state["screenSharing"] and not state["recording"])
    print("PASS: real recording/sharing, elapsed time, reload continuity, and separate stop controls")
finally:
    client.close()
    for session in sessions:
        try:
            call(session, name + ".Session", "Stop")
        except GLib.Error:
            pass
