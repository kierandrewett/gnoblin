#!/usr/bin/env python3
"""Check global launch feedback in the isolated Gnoblin session."""
import ast
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"), "Use the private Gnoblin test session"
repo = Path(__file__).resolve().parent.parent
scripts = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/scripts"
scripts.mkdir(parents=True, exist_ok=True)
shutil.copy2(repo / "src/scripts/launch-feedback.js", scripts)
(scripts / "launch-test-pointer.js").write_text('''
import Clutter from "gi://Clutter";
import GLib from "gi://GLib";
export default function enable(api) {
    const seat = global.stage.context.get_backend().get_default_seat();
    const device = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    device.notify_absolute_motion(GLib.get_monotonic_time(), 200, 200);
    api._disposers.push(() => device.run_dispose());
}
''')


def reload():
    subprocess.run([str(repo / "src/tools/gnoblinctl"), "script", "reload"], check=True)


def call(method, *args):
    result = subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnoblin.LaunchFeedback",
        "--object-path", "/org/gnoblin/LaunchFeedback", "--method", "org.gnoblin.LaunchFeedback." + method,
        *map(str, args)], check=True, capture_output=True, text=True)
    return ast.literal_eval(result.stdout)


def state():
    return json.loads(call("GetState")[0])


def wait_for(predicate, timeout=2):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        current = state()
        if predicate(current):
            return current
        time.sleep(0.03)
    raise AssertionError(current)


reload()
wait_for(lambda value: value["pointerVisible"])
if os.environ.get("GNOBLIN_EXPECT_NATIVE_CURSOR") == "1":
    assert state()["nativeCursor"], "Native cursor API not loaded"
print("Cursor backend:", "native" if state()["nativeCursor"] else "GNOME artwork fallback")
call("Begin", "one", "__missing_app__", 600)
wait_for(lambda value: value["busy"] and value["spinnerVisible"] and value["pointerVisible"] == value["nativeCursor"])
call("Begin", "two", "__missing_app__", 2000)
call("End", "one")
assert state()["pending"] == 1 and state()["busy"]
call("End", "two")
assert state()["pointerVisible"] and not state()["busy"]
call("Begin", "timeout", "__missing_app__", 200)
wait_for(lambda value: not value["busy"] and value["pointerVisible"])
call("Begin", "reload", "__missing_app__", 5000)
reload()
assert state()["pointerVisible"] and not state()["busy"]
call("Begin", "window", "gnoblin-launch-feedback-test", 5000)
started = time.monotonic()
app = subprocess.Popen(["foot", "--app-id=gnoblin-launch-feedback-test", "sleep", "30"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    wait_for(lambda value: not value["busy"] and value["pointerVisible"], timeout=4)
    assert time.monotonic() - started < 4, "Window matching must finish before the timeout"
    assert app.poll() is None, "Test application failed to open"
finally:
    app.terminate()
    app.wait(timeout=3)
    call("End", "window")
print("LAUNCH_FEEDBACK_PASSED: global cursor, overlap, timeout, reload, real application mapping")
