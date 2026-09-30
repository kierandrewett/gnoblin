#!/usr/bin/env python3
"""Exercise native launch feedback through gnoblinctl in a private session."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

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


def launches():
    return ctl("launch", "status")["launches"]


def find(token):
    return next((launch for launch in launches() if launch["token"] == token), None)


def wait_for(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        current = find(token)
        if current is not None and predicate(current):
            return current
        time.sleep(0.03)
    raise AssertionError(f"Launch {token} did not reach the expected state; current={find(token)!r}")


token = "gnoblin-test-" + uuid.uuid4().hex
second = token + "-overlap"

first_launch = ctl("launch", "begin", token, "__missing_app__", "1000")
assert first_launch["state"] == "pending", first_launch
second_launch = ctl("launch", "begin", second, "__missing_app__", "5000")
assert second_launch["state"] == "pending", second_launch

ended = ctl("launch", "end", token)
assert ended["ok"] and ended["token"] == token, ended
assert find(token)["state"] == "ended"
assert find(second)["state"] == "pending", launches()
ctl("launch", "end", second)

timeout_token = token + "-timeout"
ctl("launch", "begin", timeout_token, "__missing_app__", "200")
assert wait_for(lambda launch: launch["state"] == "timed_out", timeout=3)

reload_token = token + "-reload"
ctl("launch", "begin", reload_token, "__missing_app__", "5000")
ctl("config", "reload")
assert find(reload_token)["state"] == "pending", launches()
ctl("launch", "end", reload_token)

application = "gnoblin-launch-feedback-" + uuid.uuid4().hex
window_token = token + "-window"
ctl("launch", "begin", window_token, application, "5000")
app = subprocess.Popen(
    ["foot", f"--app-id={application}", "sleep", "30"],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
try:
    assert wait_for(lambda launch: launch["state"] == "started", timeout=4)
    assert app.poll() is None, "Test application failed to open"
finally:
    if app.poll() is None:
        app.terminate()
        app.wait(timeout=3)
    ctl("launch", "end", window_token)

print("PASS: native launch API covers overlap, explicit end, timeout, reload and mapped windows")
