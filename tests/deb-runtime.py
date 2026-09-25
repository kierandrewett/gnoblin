#!/usr/bin/env python3
"""Exercise the installed CLI and bridge in the private package-test desktop."""

import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
config = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
config.mkdir(parents=True, exist_ok=True)
(config / "init.lua").write_text(
    'gnoblin.window_rule { match = { type = "window", focused = false }, opacity = 0.95 }\n'
)


def call(*arguments):
    return json.loads(subprocess.check_output(["/usr/bin/gnoblinctl", "--json", *arguments], text=True))


def wait_for(predicate):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError("Installed package did not reach the expected window state")


call("reload")
wait_for(lambda: Path(os.environ["GNOBLIN_COMPOSITOR_SOCKET"]).exists())
assert call("feature", "list")["features"]
window = subprocess.Popen(["foot", "--app-id", "gnoblin-package-test", "sleep", "60"])
try:
    wait_for(lambda: any("gnoblin-package-test" in item["appId"] for item in call("window", "list")["windows"]))
    record = next(item for item in call("window", "list")["windows"] if "gnoblin-package-test" in item["appId"])
    call("window", "minimize", record["id"])
    wait_for(
        lambda: any(item["id"] == record["id"] and item["minimized"] for item in call("window", "list")["windows"])
    )
    call("window", "restore", record["id"])
    wait_for(
        lambda: any(item["id"] == record["id"] and not item["minimized"] for item in call("window", "list")["windows"])
    )
    call("window", "close", record["id"])
    wait_for(lambda: all(item["id"] != record["id"] for item in call("window", "list")["windows"]))
finally:
    window.terminate()
    window.wait(timeout=10)
print("PASS: installed CLI, Lua config, bridge, window listing, minimise, restore and close")
