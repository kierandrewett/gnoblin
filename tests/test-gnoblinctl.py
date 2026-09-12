#!/usr/bin/env python3
"""Run CLI window actions only in the isolated Gnoblin test desktop."""

import json
import os
from pathlib import Path
import subprocess
import shutil
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
repo = Path(__file__).resolve().parents[1]
ctl = str(repo / "src/tools/gnoblinctl")
config = Path(os.environ["XDG_CONFIG_HOME"])
script_dir = config / "gnoblin/scripts"
script_dir.mkdir(parents=True, exist_ok=True)
if (repo / "src/scripts/lib").is_dir():
    shutil.copytree(repo / "src/scripts/lib", script_dir / "lib", ignore=shutil.ignore_patterns("__pycache__"))
path = config / "compositor.sock"
bridge = (
    (repo / "src/scripts/compositor-bridge.js")
    .read_text()
    .replace("GLib.getenv('GNOBLIN_COMPOSITOR_SOCKET')", json.dumps(str(path)))
)
(script_dir / "00-cli-workspaces.js").write_text("""
import Gio from 'gi://Gio';
export default function () {
    new Gio.Settings({schema_id: 'org.gnome.mutter'}).set_boolean('dynamic-workspaces', false);
    new Gio.Settings({schema_id: 'org.gnome.desktop.wm.preferences'}).set_int('num-workspaces', 3);
}
""")
(script_dir / "compositor-bridge.js").write_text(bridge)
(script_dir / "launch-feedback.js").write_text((repo / "src/scripts/launch-feedback.js").read_text())


def call(*args):
    result = subprocess.run(
        [ctl, "--json", "--socket", str(path), *map(str, args)], capture_output=True, text=True, timeout=8
    )
    assert result.returncode == 0, (args, result.stderr)
    return json.loads(result.stdout)


def until(query, predicate):
    deadline = time.monotonic() + 6
    result = None
    while time.monotonic() < deadline:
        result = query()
        if predicate(result):
            return result
        time.sleep(0.05)
    raise AssertionError({"last": result, "windows": call("window", "list")})


call("script", "reload")
for _ in range(40):
    if path.exists():
        break
    time.sleep(0.05)
assert call("ping") == "pong"
assert call("monitor", "list")["monitors"]
assert call("feature", "list")["features"]
call("launch", "begin", "cli-private-test", "cli-test", 1000)
assert call("launch", "status")["busy"]
call("launch", "end", "cli-private-test")
window = subprocess.Popen(
    ["foot", "--app-id", "gnoblin-cli-test", "--title", "CLI 'quoted' window", "sleep", "60"],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
try:
    rows = until(lambda: call("window", "list", "--app-id", "gnoblin-cli-test")["windows"], bool)
    identity = rows[0]["id"]

    def state():
        return next(row for row in call("window", "list")["windows"] if row["id"] == identity)

    call("window", "focus", identity)
    until(state, lambda row: row["focused"])
    call("window", "minimize", identity)
    until(state, lambda row: row["minimized"])
    call("window", "restore", identity)
    until(state, lambda row: not row["minimized"])
    call("window", "maximize", identity)
    until(state, lambda row: row["maximized"])
    call("window", "unmaximize", identity)
    until(state, lambda row: not row["maximized"])
    call("window", "fullscreen", identity)
    until(state, lambda row: row["fullscreen"])
    call("window", "unfullscreen", identity)
    until(state, lambda row: not row["fullscreen"])
    time.sleep(0.3)
    call("window", "move", identity, 80, 100)
    until(state, lambda row: row["geometry"]["x"] == 80 and row["geometry"]["y"] == 100)
    call("window", "resize", identity, 600, 400)
    until(state, lambda row: abs(row["geometry"]["width"] - 600) < 20 and abs(row["geometry"]["height"] - 400) < 30)
    workspaces = call("workspace", "list")["workspaces"]
    destination = workspaces[-1]["id"]
    call("window", "workspace", identity, destination)
    until(state, lambda row: row["workspace"] == destination)
    call("workspace", "switch", destination)
    until(
        lambda: call("workspace", "list")["workspaces"],
        lambda rows: any(row["id"] == destination and row["active"] for row in rows),
    )
    call("window", "monitor", identity, 0)
    assert state()["monitorIndex"] == 0
    invalid = subprocess.run([ctl, "--socket", str(path), "window", "focus", "invalid"], capture_output=True, text=True)
    assert invalid.returncode == 1 and "no longer available" in invalid.stderr
    call("window", "close", identity)
    until(lambda: call("window", "list")["windows"], lambda rows: all(row["id"] != identity for row in rows))
    print(
        "PASS: CLI lists, focus, minimise, restore, maximise, fullscreen, geometry, workspace, monitor, stale IDs and close"
    )
finally:
    if window.poll() is None:
        window.terminate()
    window.wait(timeout=5)
