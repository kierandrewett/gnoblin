#!/usr/bin/env bash
# Verify that Lua settings and the native control API reach a fresh session.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GNOBLIN_TEST_PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
mkdir -p "$ROOT/build/tmp"
fixture_root="$(mktemp -d "$ROOT/build/tmp/devkit-e2e-config.XXXXXX")"
mkdir -p "$fixture_root/gnoblin"
trap 'rm -rf -- "$fixture_root"' EXIT
schema_dir="$fixture_root/data/glib-2.0/schemas"
mkdir -p "$schema_dir"
cat >"$schema_dir/org.gnome.desktop.input-sources.gschema.xml" <<'XML'
<schemalist>
  <schema id="org.gnome.desktop.input-sources" path="/org/gnome/desktop/input-sources/">
    <key name="sources" type="a(ss)">
      <default>[('xkb', 'us')]</default>
    </key>
    <key name="xkb-options" type="as">
      <default>[]</default>
    </key>
  </schema>
</schemalist>
XML
glib-compile-schemas "$schema_dir"

activation_protocol_dir="$(pkg-config --variable=pkgdatadir wayland-protocols)"
activation_protocol="$activation_protocol_dir/staging/xdg-activation/xdg-activation-v1.xml"
test -f "$activation_protocol"
wayland-scanner client-header "$activation_protocol" \
    "$fixture_root/xdg-activation-v1-client-protocol.h"
wayland-scanner private-code "$activation_protocol" \
    "$fixture_root/xdg-activation-v1-client-protocol.c"
xdg_shell_protocol="$activation_protocol_dir/stable/xdg-shell/xdg-shell.xml"
test -f "$xdg_shell_protocol"
wayland-scanner client-header "$xdg_shell_protocol" \
    "$fixture_root/xdg-shell-client-protocol.h"
wayland-scanner private-code "$xdg_shell_protocol" \
    "$fixture_root/xdg-shell-client-protocol.c"
read -r -a activation_cflags <<<"$(pkg-config --cflags wayland-client)"
read -r -a activation_libs <<<"$(pkg-config --libs wayland-client)"
cc "${activation_cflags[@]}" -I"$fixture_root" \
    "$ROOT/tests/focus-transfer-client.c" \
    "$fixture_root/xdg-activation-v1-client-protocol.c" \
    "$fixture_root/xdg-shell-client-protocol.c" \
    "${activation_libs[@]}" -o "$fixture_root/focus-transfer-client"

cat >"$fixture_root/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
}
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
    local status = gnoblin.runtime.status()
    assert(status.state == "running" and status.generation > 0)
    assert(not pcall(function() status.state = "restarting" end))
    print("LUA_API:runtime-status")
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
cat > "$XDG_RUNTIME_DIR/input-sources.lua" <<'LUA'
assert(#gnoblin.input.sources() == 0,
    "unconfigured input sources must not inherit GNOME GSettings")
print("INPUT_SOURCE:empty-without-lua-setting")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/input-sources.lua"
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    input_sources = {sources = {{type = "xkb", id = "us"}}},
}
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/input-sources-set.txt"
cat > "$XDG_RUNTIME_DIR/input-sources.lua" <<'LUA'
local sources = gnoblin.input.sources()
assert(#sources == 1 and sources[1].id == "us")
print("INPUT_SOURCE:configured-from-lua")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/input-sources.lua"
gnoblinctl input select xkb us > "$XDG_RUNTIME_DIR/input-source-select.txt"
gnoblinctl --json input current > "$XDG_RUNTIME_DIR/input-source-current.json"
python3 - "$XDG_RUNTIME_DIR/input-source-current.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    current = json.load(stream)

source = current.get("source")
assert current.get("available") is True, current
assert source and source.get("type") == "xkb" and source.get("id") == "us", current
assert source.get("current") is True, current
print("INPUT_SOURCE:selected-through-cli")
PY
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
}
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/input-sources-cleared.txt"
cat > "$XDG_RUNTIME_DIR/input-sources.lua" <<'LUA'
assert(#gnoblin.input.sources() == 0)
print("INPUT_SOURCE:cleared-with-lua-config")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/input-sources.lua"
cat > "$XDG_RUNTIME_DIR/lua-api.lua" <<'LUA'
assert(type(gnoblin.windows.list()) == "table")
assert(type(gnoblin.workspaces.list()) == "table")
assert(type(gnoblin.monitors.list()) == "table")
assert(type(gnoblin.focus.history()) == "table")
local status = gnoblin.runtime.status()
assert(status.state == "running" and status.generation > 0)
print("LUA_API:runtime-status")
print("LUA_API:snapshots")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/lua-api.lua"
python3 "$GNOBLIN_FOCUS_TEST_SCRIPT"
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
cat > "$XDG_RUNTIME_DIR/runtime-status.lua" <<'LUA'
local status = gnoblin.runtime.status()
print("RUNTIME_STATUS:" .. status.state .. ":" .. status.generation)
LUA
python3 - <<'PY'
import json
import os
from pathlib import Path
import signal
import subprocess
import time

gnoblinctl = os.environ["GNOBLIN_DEVKIT_CTL"]
wayland_display = os.environ["WAYLAND_DISPLAY"]
status_script = Path(os.environ["XDG_RUNTIME_DIR"]) / "runtime-status.lua"
status_script.write_text(
    'local s=gnoblin.runtime.status(); print("RUNTIME_STATUS:"..s.state..":"..s.generation)\n',
    encoding="utf-8",
)

def display_processes():
    for process in Path("/proc").iterdir():
        if not process.name.isdecimal():
            continue
        try:
            args = [arg for arg in process.joinpath("cmdline").read_bytes().decode().split("\0") if arg]
            executable = process.joinpath("exe").resolve(strict=True).name
        except OSError:
            continue
        try:
            display_index = args.index("--wayland-display")
        except ValueError:
            continue
        if display_index + 1 < len(args) and args[display_index + 1] == wayland_display:
            yield int(process.name), executable, args

def worker_and_compositor():
    worker = compositor = None
    for pid, executable, args in display_processes():
        if "--internal-runtime-worker" in args:
            worker = pid
        if executable == "gnoblin-mutter":
            compositor = pid
    return worker, compositor

def config_snapshot():
    result = subprocess.run(
        [gnoblinctl, "--timeout", "1", "--json", "config", "show"],
        check=False,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return json.loads(result.stdout)

def runtime_status():
    result = subprocess.run(
        [gnoblinctl, "--timeout", "1", "lua", str(status_script)],
        check=False,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return next(
        (line.removeprefix("RUNTIME_STATUS:") for line in result.stdout.splitlines()
         if line.startswith("RUNTIME_STATUS:")),
        None,
    )

worker_before, compositor_before = worker_and_compositor()
assert worker_before and compositor_before, (worker_before, compositor_before)
assert config_snapshot() is not None, "runtime config was unavailable before recovery"
status_before = runtime_status()
assert status_before and status_before.startswith("running:"), status_before
generation_before = int(status_before.split(":", 1)[1])
os.kill(worker_before, signal.SIGKILL)

deadline = time.monotonic() + 20
restart_observed = False
while time.monotonic() < deadline:
    status = runtime_status()
    if status and status.startswith("restarting:"):
        assert int(status.split(":", 1)[1]) == generation_before, status
        restart_observed = True
        break
    time.sleep(0.01)
if not restart_observed:
    raise AssertionError("runtime.status() did not report worker recovery")

deadline = time.monotonic() + 20
while time.monotonic() < deadline:
    worker_after, compositor_after = worker_and_compositor()
    status = runtime_status()
    if (
        worker_after
        and worker_after != worker_before
        and compositor_after == compositor_before
        and status == f"running:{generation_before}"
    ):
        os.kill(compositor_before, 0)
        print("WORKER:recovered-with-compositor-alive")
        break
    time.sleep(0.1)
else:
    raise AssertionError("Lua worker did not recover with the compositor alive")
PY
SCRIPT
)

# The headless host used by CI may not provide a PipeWire server for Mutter's
# optional devkit viewer. Keep the nested compositor alive if that viewer exits
# so this test isolates session-supervisor recovery.
output="$(GNOBLIN_DEVKIT_KEEP_SESSION=1 \
    GNOBLIN_STATE_DIR="$fixture_root/state" \
    XDG_DATA_DIRS="$fixture_root/data${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}:/usr/local/share:/usr/share" \
    GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$fixture_root" \
    GNOBLIN_RUNTIME_BIN="$ROOT/build/ninja/gnoblin" \
    GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
    GNOBLIN_FOCUS_TEST_CLIENT="$fixture_root/focus-transfer-client" \
    GNOBLIN_FOCUS_TEST_SCRIPT="$ROOT/tests/test-focus-transfer.py" \
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
grep -q 'INPUT_SOURCE:empty-without-lua-setting' <<<"$output"
grep -q 'INPUT_SOURCE:configured-from-lua' <<<"$output"
grep -q 'INPUT_SOURCE:selected-through-cli' <<<"$output"
grep -q 'INPUT_SOURCE:cleared-with-lua-config' <<<"$output"
grep -q 'LUA_API:runtime-status' <<<"$output"
grep -q 'LUA_API:snapshots' <<<"$output"
grep -q 'PASS: Gnoblin denied activation without user context and emitted the denial event' <<<"$output"
grep -q 'restarting Lua runtime worker' "$fixture_root/state/devkit-last.log"
printf '%s\n' 'PASS: Lua config and native control API work in the supervised nested runtime'

guardian_fixture="$fixture_root/supervisor-config"
guardian_marker="$fixture_root/supervisor-autostart.log"
mkdir -p "$guardian_fixture/gnoblin"
cat >"$guardian_fixture/gnoblin/init.lua" <<LUA
gnoblin.configure {
    autostart = {
        recovery_marker = {
            command = {"sh", "-c", "printf x >> '$guardian_marker'"},
        },
    },
}
LUA

guardian_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
python3 - <<'PY'
import json
import os
from pathlib import Path
import signal
import subprocess
import time

gnoblinctl = os.environ["GNOBLIN_DEVKIT_CTL"]
wayland_display = os.environ["WAYLAND_DISPLAY"]
marker = Path(os.environ["GNOBLIN_AUTOSTART_MARKER"])
status_script = Path(os.environ["XDG_RUNTIME_DIR"]) / "runtime-status.lua"
status_script.write_text(
    'local s=gnoblin.runtime.status(); print("RUNTIME_STATUS:"..s.state..":"..s.generation)\n',
    encoding="utf-8",
)

def display_processes():
    for process in Path("/proc").iterdir():
        if not process.name.isdecimal():
            continue
        try:
            args = [arg for arg in process.joinpath("cmdline").read_bytes().decode().split("\0") if arg]
            executable = process.joinpath("exe").resolve(strict=True).name
        except OSError:
            continue
        try:
            display_index = args.index("--wayland-display")
        except ValueError:
            continue
        if display_index + 1 < len(args) and args[display_index + 1] == wayland_display:
            yield int(process.name), executable, args

def processes():
    compositor = supervisor = None
    for pid, executable, args in display_processes():
        if "--internal-session-supervisor" in args:
            supervisor = pid
        if executable == "gnoblin-mutter":
            compositor = pid
    return compositor, supervisor

def config_snapshot():
    result = subprocess.run(
        [gnoblinctl, "--timeout", "1", "--json", "config", "show"],
        check=False,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return json.loads(result.stdout)

def runtime_status():
    result = subprocess.run(
        [gnoblinctl, "--timeout", "1", "lua", str(status_script)],
        check=False,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return next(
        (line.removeprefix("RUNTIME_STATUS:") for line in result.stdout.splitlines()
         if line.startswith("RUNTIME_STATUS:")),
        None,
    )

deadline = time.monotonic() + 15
while time.monotonic() < deadline and (not marker.exists() or config_snapshot() is None):
    time.sleep(0.05)
assert marker.read_text() == "x", marker.read_text()
compositor_before, supervisor_before = processes()
config_before = config_snapshot()
assert compositor_before and supervisor_before and config_before is not None, (
    compositor_before, supervisor_before, config_before
)
status_before = runtime_status()
assert status_before and status_before.startswith("running:"), status_before
generation_before = int(status_before.split(":", 1)[1])
os.kill(supervisor_before, signal.SIGKILL)

deadline = time.monotonic() + 30
while time.monotonic() < deadline:
    compositor_after, supervisor_after = processes()
    config_after = config_snapshot()
    status = runtime_status()
    if (
        supervisor_after
        and supervisor_after != supervisor_before
        and compositor_after == compositor_before
        and config_after == config_before
        and status == f"running:{generation_before}"
    ):
        assert marker.read_text() == "x", marker.read_text()
        os.kill(compositor_before, 0)
        print("SUPERVISOR:recovered-with-compositor-alive")
        print("AUTOSTART:ran-once-across-supervisor-recovery")
        break
    time.sleep(0.1)
else:
    raise AssertionError("session supervisor did not recover with the compositor alive")
PY
SCRIPT
)

guardian_output="$(GNOBLIN_DEVKIT_KEEP_SESSION=1 \
    GNOBLIN_STATE_DIR="$fixture_root/guardian-state" \
    GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$guardian_fixture" \
    GNOBLIN_RUNTIME_BIN="$ROOT/build/ninja/gnoblin" \
    GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
    GNOBLIN_AUTOSTART_MARKER="$guardian_marker" \
    GNOBLIN_DEVKIT_EXEC="$guardian_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$guardian_output" >&2
    exit 1
}
grep -q 'SUPERVISOR:recovered-with-compositor-alive' <<<"$guardian_output"
grep -q 'AUTOSTART:ran-once-across-supervisor-recovery' <<<"$guardian_output"
printf '%s\n' 'PASS: session supervisor recovers without restarting Mutter or login autostart'
