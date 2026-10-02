#!/usr/bin/env bash
# Verify that Lua settings and the native control API reach a fresh session.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/build/tmp"
fixture_root="$(mktemp -d "$ROOT/build/tmp/devkit-e2e-config.XXXXXX")"
mkdir -p "$fixture_root/gnoblin"
trap 'rm -rf -- "$fixture_root"' EXIT
cat >"$fixture_root/gnoblin/init.lua" <<'LUA'
gnoblin.configure {window_management = {focus_mode = "click"}}
gnoblin.events.once("gnoblin.config.reloaded", function()
    assert(type(gnoblin.settings) == "userdata")
    assert(gnoblin.settings.window_management.focus_mode == "click")
    assert(not pcall(function()
        gnoblin.settings.window_management.focus_mode = "sloppy"
    end))
    assert(type(gnoblin.windows.list()) == "table")
    assert(type(gnoblin.workspaces.list()) == "table")
    assert(type(gnoblin.monitors.list()) == "table")
    assert(type(gnoblin.focus.history()) == "table")
    print("LUA_API:snapshots")
end)
LUA

devkit_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
printf 'PING:%s\n' "$(gnoblinctl ping)"
gnoblinctl --json config show > "$XDG_RUNTIME_DIR/config.json"
python3 - "$XDG_RUNTIME_DIR/config.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    config = json.load(stream)

def contains_click(value):
    if isinstance(value, dict):
        return any(contains_click(item) for item in value.values())
    if isinstance(value, list):
        return any(contains_click(item) for item in value)
    return value == "click"

assert contains_click(config), config
print("CONFIG:click")
PY
gnoblinctl config reload > "$XDG_RUNTIME_DIR/config-reload.txt"
gnoblinctl --json window list > "$XDG_RUNTIME_DIR/windows.json"
python3 - "$XDG_RUNTIME_DIR/windows.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    json.load(stream)
print("WINDOWS:json")
PY
gnoblinctl --json workspace next > "$XDG_RUNTIME_DIR/workspace-next.json"
python3 - "$XDG_RUNTIME_DIR/workspace-next.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    json.load(stream)
print("WORKSPACE:next")
PY
python3 - <<'PY'
import json
import os
from pathlib import Path
import signal
import subprocess
import time

host_pid = int(os.environ["GNOBLIN_DEVKIT_HOST_PID"])
gnoblinctl = os.environ["GNOBLIN_DEVKIT_CTL"]

def children(pid):
    path = Path(f"/proc/{pid}/task/{pid}/children")
    return [int(child) for child in path.read_text().split()]

def arguments(pid):
    return Path(f"/proc/{pid}/cmdline").read_bytes().decode().split("\0")

def worker_and_compositor():
    worker = compositor = None
    for pid in children(host_pid):
        args = arguments(pid)
        if "--internal-runtime-worker" in args:
            worker = pid
        if any(arg.startswith("--gnoblin-runtime-fd=") for arg in args):
            compositor = pid
    return worker, compositor

worker_before, compositor_before = worker_and_compositor()
assert worker_before and compositor_before, (worker_before, compositor_before)
os.kill(worker_before, signal.SIGKILL)

deadline = time.monotonic() + 20
while time.monotonic() < deadline:
    worker_after, compositor_after = worker_and_compositor()
    if worker_after and worker_after != worker_before:
        response = subprocess.run(
            [gnoblinctl, "--timeout", "1", "--json", "config", "show"],
            check=False,
            capture_output=True,
            text=True,
            timeout=3,
        )
        if response.returncode == 0:
            json.loads(response.stdout)
            assert compositor_after == compositor_before, (compositor_before, compositor_after)
            os.kill(compositor_before, 0)
            print("WORKER:recovered-with-compositor-alive")
            break
    time.sleep(0.1)
else:
    raise AssertionError("Lua worker did not recover with the compositor alive")
PY
SCRIPT
)

output="$(GNOBLIN_STATE_DIR="$fixture_root/state" \
    GNOBLIN_PREFIX="$ROOT/install" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$fixture_root" \
    GNOBLIN_RUNTIME_BIN="$ROOT/build/ninja/gnoblin" \
    GNOBLIN_DEVKIT_CTL="$ROOT/install/bin/gnoblinctl" \
    GNOBLIN_DEVKIT_EXEC="$devkit_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$output" >&2
    exit 1
}
grep -q 'Gnoblin is ready on nested Wayland display' <<<"$output"
grep -q 'PING:pong' <<<"$output"
grep -q 'CONFIG:click' <<<"$output"
grep -q 'WINDOWS:json' <<<"$output"
grep -q 'WORKSPACE:next' <<<"$output"
grep -q 'WORKER:recovered-with-compositor-alive' <<<"$output"
grep -q 'restarting Lua runtime worker' "$fixture_root/state/devkit-last.log"
if ! grep -q 'LUA_API:snapshots' "$fixture_root/state/devkit-last.log"; then
    tail -n 60 "$fixture_root/state/devkit-last.log" >&2
    exit 1
fi
printf '%s\n' 'PASS: Lua config and native control API work in the supervised nested runtime'
