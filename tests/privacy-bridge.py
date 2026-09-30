#!/usr/bin/env python3
"""Exercise native privacy snapshots against real Mutter ScreenCast sessions."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402 - Select GI versions before importing their modules.

ROOT = Path(__file__).resolve().parents[1]
GNOBLINCTL = Path(os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or ROOT / "build/ninja/gnoblinctl")
assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"


def ctl(*arguments):
    result = subprocess.run(
        [str(GNOBLINCTL), "--json", *arguments],
        check=True,
        capture_output=True,
        text=True,
        timeout=15,
    )
    return json.loads(result.stdout)


def privacy_state():
    return ctl("privacy")


def wait_for(predicate, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        current = privacy_state()
        if predicate(current):
            return current
        time.sleep(0.05)
    raise AssertionError(f"Privacy state did not match; current={privacy_state()!r}")


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


def start(recording):
    path = call("/org/gnome/Mutter/ScreenCast", name, "CreateSession", "(a{sv})", ({},))[0]
    properties = {
        "cursor-mode": GLib.Variant("u", 0),
        "is-recording": GLib.Variant("b", recording),
    }
    call(path, name + ".Session", "RecordMonitor", "(sa{sv})", ("", properties))
    call(path, name + ".Session", "Start")
    return path


sessions = []
try:
    initial = privacy_state()
    assert initial["available"]["screen_sharing"] is True, initial
    assert initial["available"]["recording"] is True, initial
    assert initial["screen_sharing"] is False and initial["recording"] is False, initial

    sessions.append(start(True))
    wait_for(lambda state: state["recording"] is True)
    sessions.append(start(False))
    wait_for(lambda state: state["screen_sharing"] is True and state["recording"] is True)

    ctl("config", "reload")
    after_reload = wait_for(lambda state: state["screen_sharing"] and state["recording"])
    assert after_reload["revision"] >= initial["revision"], after_reload

    stopped_sharing = ctl("privacy", "stop-sharing")
    assert stopped_sharing["requested"] >= 1, stopped_sharing
    wait_for(lambda state: not state["screen_sharing"] and state["recording"])

    stopped_recording = ctl("privacy", "stop-recording")
    assert stopped_recording["requested"] >= 1, stopped_recording
    wait_for(lambda state: not state["screen_sharing"] and not state["recording"])
    print("PASS: native privacy snapshots, reload continuity and separate stop controls")
finally:
    for session in sessions:
        try:
            call(session, name + ".Session", "Stop")
        except GLib.Error:
            pass
