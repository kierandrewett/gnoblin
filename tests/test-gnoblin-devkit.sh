#!/usr/bin/env bash
# Verify that Lua settings and the native control API reach a fresh session.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export GNOBLIN_TEST_ROOT="$ROOT"
GNOBLIN_TEST_PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
if [[ -n ${GNOBLIN_RUNTIME_BIN:-} ]]; then
    GNOBLIN_TEST_RUNTIME="$GNOBLIN_RUNTIME_BIN"
elif [[ -n ${GNOBLIN_PREFIX:-} ]]; then
    GNOBLIN_TEST_RUNTIME="$GNOBLIN_TEST_PREFIX/bin/gnoblin"
else
    GNOBLIN_TEST_RUNTIME="$ROOT/build/ninja/gnoblin"
fi
[[ -x "$GNOBLIN_TEST_RUNTIME" ]] || {
    echo "No Gnoblin runtime executable found: $GNOBLIN_TEST_RUNTIME" >&2
    exit 1
}
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
    input = {
        mouse = {drag_threshold = 24},
    },
}
gnoblin.events.once("gnoblin.config.reloaded", function(event)
    assert(type(event.sequence) == "number" and event.sequence > 0)
    assert(type(event.time) == "number" and event.time > 0)
    assert(type(gnoblin.settings) == "userdata")
    assert(gnoblin.settings.window_management.focus_mode == "click")
    assert(gnoblin.settings.input.mouse.drag_threshold == 24)
    assert(not pcall(function()
        gnoblin.settings.window_management.focus_mode = "sloppy"
    end))
    assert(type(gnoblin.windows.list()) == "table")
    assert(type(gnoblin.workspaces.list()) == "table")
    assert(type(gnoblin.monitors.list()) == "table")
    assert(type(gnoblin.focus.history()) == "table")
    local session_status = gnoblin.session.status()
    assert(session_status.state == "running")
    assert(type(session_status.revision) == "number" and session_status.revision >= 0)
    assert(not pcall(function() session_status.revision = 0 end))
    local status = gnoblin.runtime.status()
    assert(status.state == "running" and status.generation > 0)
    assert(not pcall(function() status.state = "restarting" end))
    print("LUA_API:runtime-status")
    print("LUA_API:snapshots")
end)
gnoblin.events.once("gnoblin.session.activity-changed", function(event)
    local activity = gnoblin.session.activity()
    assert(type(event.revision) == "number" and event.revision > 0)
    assert(activity.revision >= event.revision,
        "session activity snapshot must be current before its event is delivered")
    print("LUA_API:activity-event-snapshot")
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
source "$GNOBLIN_TEST_ROOT/scripts/gnoblin-test-ibus.sh"
ibus_pid_file="$GNOBLIN_TEST_IBUS_PID_FILE"
ibus_log_file="$GNOBLIN_TEST_IBUS_LOG_FILE"
trap 'gnoblin_test_ibus_stop "$ibus_pid_file"' EXIT
select_ibus_source() {
    local output_file="$1"
    local selected=false
    for _ in {1..100}; do
        if gnoblinctl input select ibus xkb:us::eng >"$output_file" 2>&1; then
            selected=true
            break
        fi
        sleep 0.1
    done
    if [[ "$selected" != true ]]; then
        cat "$output_file" >&2
        echo 'Gnoblin could not select the configured IBus engine' >&2
        exit 1
    fi
}
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "ibus", id = "xkb:us::eng"},
        },
    },
}
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/input-sources-set.txt"
cat > "$XDG_RUNTIME_DIR/mouse-threshold-reset.lua" <<'LUA'
local input = gnoblin.settings.input
assert(not input or not input.mouse or input.mouse.drag_threshold == nil)
print("INPUT:mouse-drag-threshold-inherited")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/mouse-threshold-reset.lua"
cat > "$XDG_RUNTIME_DIR/input-sources.lua" <<'LUA'
local sources = gnoblin.input.sources()
assert(#sources == 2 and sources[1].id == "us")
assert(sources[2].type == "ibus" and sources[2].id == "xkb:us::eng")
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
select_ibus_source "$XDG_RUNTIME_DIR/ibus-source-select.txt"
cat > "$XDG_RUNTIME_DIR/ibus-current.lua" <<'LUA'
local current = gnoblin.input.current_source()
assert(current and current.type == "ibus" and current.id == "xkb:us::eng")
print("IBUS:selected-through-cli")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-current.lua"
gnoblin_test_ibus_stop "$ibus_pid_file"
cat > "$XDG_RUNTIME_DIR/ibus-lost.lua" <<'LUA'
local current = gnoblin.input.current_source()
assert(not current or current.type ~= "ibus")
print("IBUS:owner-lost")
LUA
ibus_lost=false
for _ in {1..100}; do
    if gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-lost.lua" \
        > "$XDG_RUNTIME_DIR/ibus-lost.txt" 2>&1; then
        ibus_lost=true
        break
    fi
    sleep 0.1
done
if [[ "$ibus_lost" != true ]]; then
    cat "$XDG_RUNTIME_DIR/ibus-lost.txt" >&2
    echo 'IBus source did not clear after its owner exited' >&2
    exit 1
fi
cat "$XDG_RUNTIME_DIR/ibus-lost.txt"
DISPLAY='' WAYLAND_DISPLAY="$GNOBLIN_TEST_IBUS_WAYLAND_DISPLAY" \
    gnoblin_test_ibus_start "$ibus_pid_file" "$ibus_log_file"
select_ibus_source "$XDG_RUNTIME_DIR/ibus-source-reselect.txt"
gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-current.lua"
printf 'IBUS:reconnected-after-owner-restart\n'
for reload_attempt in {1..8}; do
    gnoblinctl config reload > "$XDG_RUNTIME_DIR/reload-$reload_attempt.txt"
done
printf 'CONFIG_RELOAD:stable\n'
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
    GNOBLIN_TEST_IBUS_DAEMON=1 \
    GNOBLIN_STATE_DIR="$fixture_root/state" \
    XDG_DATA_DIRS="$fixture_root/data${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}:/usr/local/share:/usr/share" \
    GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$fixture_root" \
    GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
    GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
    GNOBLIN_FOCUS_TEST_CLIENT="$fixture_root/focus-transfer-client" \
    GNOBLIN_FOCUS_TEST_SCRIPT="$ROOT/tests/test-focus-transfer.py" \
    GNOBLIN_DEVKIT_EXEC="$devkit_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$output" >&2
    exit 1
}
require_output() {
    local expected="$1"
    if ! grep -Fq -- "$expected" <<<"$output"; then
        printf 'Missing expected devkit output: %s\n' "$expected" >&2
        printf '%s\n' 'Captured devkit output:' "$output" >&2
        exit 1
    fi
}
require_output 'Gnoblin is ready on nested Wayland display'
require_output 'PING:pong'
require_output 'CONFIG:click'
require_output 'WINDOWS:json'
require_output 'WORKSPACE:next'
require_output 'WORKER:recovered-with-compositor-alive'
require_output 'INPUT_SOURCE:empty-without-lua-setting'
require_output 'INPUT_SOURCE:configured-from-lua'
require_output 'INPUT_SOURCE:selected-through-cli'
require_output 'INPUT:mouse-drag-threshold-inherited'
require_output 'IBUS:selected-through-cli'
require_output 'IBUS:owner-lost'
require_output 'IBUS:reconnected-after-owner-restart'
require_output 'INPUT_SOURCE:cleared-with-lua-config'
require_output 'CONFIG_RELOAD:stable'
require_output 'LUA_API:runtime-status'
require_output 'LUA_API:snapshots'
require_output 'PASS: Gnoblin denied activation without user context and emitted the denial event'
if ! grep -Fq 'LUA_API:activity-event-snapshot' "$fixture_root/state/devkit-last.log"; then
    echo 'Missing Lua session activity event proof in the devkit runtime log' >&2
    tail -n 80 "$fixture_root/state/devkit-last.log" >&2
    exit 1
fi
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
session_status_script = Path(os.environ["XDG_RUNTIME_DIR"]) / "session-status.lua"
session_status_script.write_text(
    'local s=gnoblin.session.status(); '
    'assert(s.state == "running" and type(s.revision) == "number" and s.revision >= 0); '
    'print("SESSION_STATUS:"..s.state..":"..s.revision)\n',
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

def session_status():
    result = subprocess.run(
        [gnoblinctl, "--timeout", "1", "lua", str(session_status_script)],
        check=False,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return next(
        (line.removeprefix("SESSION_STATUS:") for line in result.stdout.splitlines()
         if line.startswith("SESSION_STATUS:")),
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
os.kill(supervisor_before, signal.SIGSTOP)
try:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        status = Path(f"/proc/{supervisor_before}/status").read_text(encoding="utf-8")
        if next(line for line in status.splitlines() if line.startswith("State:"))[7] in "Tt":
            break
        time.sleep(0.01)
    else:
        raise AssertionError("session supervisor did not stop for the status read")
    status = session_status()
    assert status and status.startswith("running:"), status
    print("SESSION_STATUS:available-with-supervisor-stopped")
finally:
    os.kill(supervisor_before, signal.SIGCONT)
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
    GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
    GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
    GNOBLIN_AUTOSTART_MARKER="$guardian_marker" \
    GNOBLIN_DEVKIT_EXEC="$guardian_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$guardian_output" >&2
    exit 1
}
grep -q 'SUPERVISOR:recovered-with-compositor-alive' <<<"$guardian_output"
grep -q 'AUTOSTART:ran-once-across-supervisor-recovery' <<<"$guardian_output"
grep -q 'SESSION_STATUS:available-with-supervisor-stopped' <<<"$guardian_output"
printf '%s\n' 'PASS: session supervisor recovers without restarting Mutter or login autostart'
