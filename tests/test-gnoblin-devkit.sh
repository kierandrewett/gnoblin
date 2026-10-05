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
command -v grim >/dev/null || {
    echo "Missing required screenshot tool: grim" >&2
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
foreign_toplevel_protocol="$ROOT/src/protocols/foreign-toplevel-management/wlr-foreign-toplevel-management-unstable-v1.xml"
wayland-scanner client-header "$foreign_toplevel_protocol" \
    "$fixture_root/wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"
wayland-scanner private-code "$foreign_toplevel_protocol" \
    "$fixture_root/wlr-foreign-toplevel-management-unstable-v1-protocol.c"
read -r -a activation_cflags <<<"$(pkg-config --cflags wayland-client)"
read -r -a activation_libs <<<"$(pkg-config --libs wayland-client)"
cc "${activation_cflags[@]}" -I"$fixture_root" \
    "$ROOT/tests/focus-transfer-client.c" \
    "$fixture_root/xdg-activation-v1-client-protocol.c" \
    "$fixture_root/xdg-shell-client-protocol.c" \
    "${activation_libs[@]}" -o "$fixture_root/focus-transfer-client"
cc "${activation_cflags[@]}" -I"$fixture_root" \
    "$ROOT/tests/input-source-focus-client.c" \
    "$fixture_root/xdg-shell-client-protocol.c" \
    "$fixture_root/wlr-foreign-toplevel-management-unstable-v1-protocol.c" \
    "${activation_libs[@]}" -o "$fixture_root/input-source-focus-client"

layer_shell_protocol="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
test -f "$layer_shell_protocol"
wayland-scanner client-header "$layer_shell_protocol" \
    "$fixture_root/wlr-layer-shell-unstable-v1-client-protocol.h"
wayland-scanner private-code "$layer_shell_protocol" \
    "$fixture_root/wlr-layer-shell-unstable-v1-protocol.c"
xdg_shell_protocol="$activation_protocol_dir/stable/xdg-shell/xdg-shell.xml"
wayland-scanner private-code "$xdg_shell_protocol" "$fixture_root/xdg-shell-protocol.c"
read -r -a layer_cflags <<<"$(pkg-config --cflags wayland-client)"
read -r -a layer_libs <<<"$(pkg-config --libs wayland-client)"
cc "${layer_cflags[@]}" -I"$fixture_root" \
    "$ROOT/tests/layer-lifecycle-lua-client.c" \
    "$fixture_root/wlr-layer-shell-unstable-v1-protocol.c" \
    "$fixture_root/xdg-shell-protocol.c" \
    "${layer_libs[@]}" -o "$fixture_root/layer-lifecycle-lua-client"

cat >"$fixture_root/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
    },
    input = {
        mouse = {
            double_click_time = 350,
            drag_threshold = 24,
            middle_click_emulation = true,
        },
        touchpad = {
            middle_click_emulation = false,
            disable_while_typing_timeout = 500,
        },
        keyboard = {
            accessibility = {
                shortcuts_enabled = false,
                slow_keys = {enabled = false, delay_ms = 400},
            },
        },
        tablets = {
            ["056a:00b9"] = {area = {0.05, 0.1, 0.15, 0.2}},
        },
    },
    workspaces = {
        {id = "main", name = "Main"},
        {id = "chat", name = "Chat"},
    },
}
gnoblin.events.on("gnoblin.session.state-changed", function(event)
    assert(event.state == "starting" or event.state == "running" or event.state == "stopping")
    local status = gnoblin.session.status()
    assert(status.session_state == event.state)
    assert(status.session_revision == event.revision)
    print("SESSION_LIFECYCLE:" .. event.state)
end)
gnoblin.events.once("gnoblin.config.reloaded", function(event)
    assert(type(event.sequence) == "number" and event.sequence > 0)
    assert(type(event.time) == "number" and event.time > 0)
    assert(type(gnoblin.settings) == "userdata")
    assert(gnoblin.settings.window_management.focus_mode == "click")
    assert(gnoblin.settings.input.mouse.drag_threshold == 24)
    assert(gnoblin.settings.input.mouse.middle_click_emulation == true)
    assert(gnoblin.settings.input.touchpad.middle_click_emulation == false)
    assert(gnoblin.settings.input.touchpad.disable_while_typing_timeout == 500)
    local area = gnoblin.settings.input.tablets["056a:00b9"].area
    assert(#area == 4 and area[1] == 0.05 and area[2] == 0.1 and
        area[3] == 0.15 and area[4] == 0.2)
    assert(not pcall(function()
        gnoblin.settings.window_management.focus_mode = "sloppy"
    end))
    assert(type(gnoblin.windows.list()) == "table")
    assert(type(gnoblin.monitors.list()) == "table")
    assert(type(gnoblin.focus.history()) == "table")
    local session_status = gnoblin.session.status()
    assert(session_status.state == "running")
    assert(type(session_status.revision) == "number" and session_status.revision >= 0)
    assert(session_status.session_state == "running")
    assert(type(session_status.session_revision) == "number" and session_status.session_revision > 0)
    assert(not pcall(function() session_status.revision = 0 end))
    local status = gnoblin.runtime.status()
    assert(status.state == "running" and status.generation > 0)
    assert(not pcall(function() status.state = "restarting" end))
    print("LUA_API:runtime-status")
    print("LUA_API:snapshots")
end)
gnoblin.events.on("gnoblin.shortcut.session.activated", function(event)
    if event.id == "shell_input_capture" then
        assert(type(event.session_id) == "number" and event.session_id > 0)
        assert(event.trigger == "release")
        print("SHORTCUT_CAPTURE:session-activated")
    end
end)
gnoblin.events.on("gnoblin.shortcut.session.key", function(event)
    if event.id == "shell_input_capture" then
        assert(event.phase == "press" or event.phase == "release")
        print("SHORTCUT_CAPTURE:key")
    end
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
workspace_names="$(gsettings get org.gnome.desktop.wm.preferences workspace-names)"
if [[ ! "$workspace_names" =~ ^(@as )?\[\]$ ]]; then
    printf 'Lua workspace config changed GNOME GSettings: %s\n' "$workspace_names" >&2
    exit 1
fi
printf 'GSETTINGS:workspace-names-untouched\n'
cat > "$XDG_RUNTIME_DIR/mouse-settings.lua" <<'LUA'
assert(gnoblin.settings.input.mouse.double_click_time == 350)
print("LUA_API:mouse-double-click-time")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/mouse-settings.lua"
cat > "$XDG_RUNTIME_DIR/tablet-area.lua" <<'LUA'
local area = gnoblin.settings.input.tablets["056a:00b9"].area
assert(#area == 4 and area[1] == 0.05 and area[2] == 0.1 and
    area[3] == 0.15 and area[4] == 0.2)
print("LUA_API:tablet-active-area-config")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/tablet-area.lua"
cat > "$XDG_RUNTIME_DIR/workspaces.lua" <<'LUA'
local workspaces = gnoblin.workspaces.list()
assert(#workspaces == 2)
assert(workspaces[1].id == "main" and workspaces[1].name == "Main")
assert(workspaces[2].id == "chat" and workspaces[2].name == "Chat")
print("LUA_API:workspace-config")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/workspaces.lua"
cat > "$XDG_RUNTIME_DIR/shortcuts.lua" <<'LUA'
for _, shortcut in ipairs(gnoblin.shortcuts.list()) do
    assert(shortcut.name ~= "shell_input_capture",
        "capture-only config must be represented through shortcut-session events")
end
print("LUA_API:configured-shortcut-input-capture")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/shortcuts.lua"
cat > "$XDG_RUNTIME_DIR/shortcut-actions.lua" <<'LUA'
local available_actions = {}
for _, action in ipairs(gnoblin.shortcuts.actions("wm")) do
    available_actions[action.id] = true
end
assert(available_actions["wm.close"], "handler-backed Mutter actions must be listed")
assert(not available_actions["wm.panel_run_dialog"],
    "actions without an executable Mutter handler must be omitted")
print("LUA_API:executable-shortcut-actions")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/shortcut-actions.lua"
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
cat > "$XDG_RUNTIME_DIR/input-accessibility.lua" <<'LUA'
local accessibility = gnoblin.settings.input.keyboard.accessibility
assert(accessibility.shortcuts_enabled == false)
assert(accessibility.slow_keys.enabled == false)
assert(accessibility.slow_keys.delay_ms == 400)
print("LUA_API:input-accessibility-config")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/input-accessibility.lua"
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
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
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
gnoblin_test_ibus_select_source gnoblinctl "$XDG_RUNTIME_DIR/ibus-source-select.txt"
cat > "$XDG_RUNTIME_DIR/ibus-current.lua" <<'LUA'
local current = gnoblin.input.current_source()
assert(current and current.type == "ibus" and current.id == "xkb:us::eng")
print("IBUS:selected-through-cli")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-current.lua"
gnoblinctl input select xkb us > "$XDG_RUNTIME_DIR/xkb-source-after-ibus.txt"
cat > "$XDG_RUNTIME_DIR/xkb-current.lua" <<'LUA'
local current = gnoblin.input.current_source()
assert(current and current.type == "xkb" and current.id == "us")
print("INPUT_SOURCE:ibus-to-xkb")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/xkb-current.lua"
gnoblin_test_ibus_select_source gnoblinctl "$XDG_RUNTIME_DIR/ibus-source-reselected.txt"
gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-current.lua"
printf 'INPUT_SOURCE:xkb-to-ibus\n'
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
gnoblin_test_ibus_select_source gnoblinctl "$XDG_RUNTIME_DIR/ibus-source-reselect.txt"
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
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
    },
    xwayland = {
        scaling_factor = 2,
    },
}
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/xwayland-scale-set.txt"
GNOBLIN_EXPECTED_SCALE=2 \
    python3 "$GNOBLIN_TEST_ROOT/tests/test-xwayland-xsettings.py"
printf 'XWAYLAND:xsettings-live-scaling-selection-loss\n'
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
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
python3 - <<'PY'
import os
from pathlib import Path
import subprocess
import time

runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
ctl = os.environ["GNOBLIN_DEVKIT_CTL"]
client_binary = os.environ["GNOBLIN_FOCUS_TEST_CLIENT"]
event_script = runtime_dir / "mutter-event.lua"
request_path = runtime_dir / "mutter-event-client-request"
request_path.unlink(missing_ok=True)
event_script.write_text(
    'gnoblin.events.mutter.once("mutter.display.window-created", function(event)\n'
    '  assert(event.event == "mutter.display.window-created")\n'
    '  assert(event.source == "display" and event.signal == "window-created")\n'
    '  assert(type(event.sequence) == "number" and event.sequence > 0)\n'
    '  assert(type(event.time) == "number" and event.time > 0)\n'
    '  assert(type(event.arg0) == "table" and type(event.arg0.window_id) == "number")\n'
    '  print("CLI_MUTTER_EVENT:display.window-created")\n'
    'end)\n',
    encoding="utf-8",
)
listener = subprocess.Popen(
    [ctl, "--timeout", "1", "lua", str(event_script)],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
)
client = None
try:
    time.sleep(0.25)
    if listener.poll() is not None:
        stdout, stderr = listener.communicate()
        raise AssertionError(f"Mutter event listener exited before a client was started: {stdout}\n{stderr}")

    client = subprocess.Popen(
        [client_binary, str(request_path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        stdout, stderr = listener.communicate(timeout=10)
    except subprocess.TimeoutExpired as error:
        raise AssertionError("gnoblinctl lua did not receive Mutter display.window-created") from error
    assert listener.returncode == 0, f"Mutter event listener failed: {stdout}\n{stderr}"
    assert "CLI_MUTTER_EVENT:display.window-created" in stdout, stdout
    print("CLI_MUTTER_EVENT:socket-forwarded")
finally:
    request_path.unlink(missing_ok=True)
    for process in (client, listener):
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)
PY
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
    },
}
local function report_workspace_animation(event)
    if event.event == "workspace-switch" then
        assert(event.name == "gnoblin.animation.started" or
            event.name == "gnoblin.animation.finished")
        assert(type(event.from_workspace) == "string" and event.from_workspace ~= "")
        assert(type(event.to_workspace) == "string" and event.to_workspace ~= "")
        assert(event.target == event.to_workspace)
        assert(event.direction == "left" or event.direction == "right" or
            event.direction == "up" or event.direction == "down" or
            event.direction == "up-left" or event.direction == "up-right" or
            event.direction == "down-left" or event.direction == "down-right")
        print("LUA_WORKSPACE_ANIMATION:" .. event.name)
    end
end
gnoblin.events.on("gnoblin.animation.started", report_workspace_animation)
gnoblin.events.on("gnoblin.animation.finished", report_workspace_animation)
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/config-reload.txt"
python3 "$GNOBLIN_TEST_ROOT/tests/test-workspace-animation-events.py"
python3 "$GNOBLIN_TEST_ROOT/tests/test-resize-animation-events.py"
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
    },
}
local namespace = "gnoblin-lua-layer-lifecycle-e2e"
local function layer_from_snapshot(id)
    for _, layer in ipairs(gnoblin.layers.list()) do
        if layer.id == id then return layer end
    end
end
gnoblin.events.on("gnoblin.layer.created", function(event)
    if event.layer.namespace ~= namespace then return end
    local current = layer_from_snapshot(event.layer.id)
    assert(current and current.namespace == namespace,
        "created callback must see its layer in gnoblin.layers.list()")
    assert(current.mapped == false, "new layer role should first appear unmapped")
    print("LUA_LAYER:created-snapshot")
end)
gnoblin.events.on("gnoblin.layer.changed", function(event)
    if event.layer.namespace ~= namespace then return end
    local saw_mapped = false
    for _, field in ipairs(event.changed) do
        if field == "mapped" then saw_mapped = true end
    end
    if not saw_mapped then return end
    local current = layer_from_snapshot(event.layer_id)
    assert(current and current.mapped == event.layer.mapped,
        "mapped callback must see the updated state in gnoblin.layers.list()")
    print(current.mapped and "LUA_LAYER:mapped-snapshot" or "LUA_LAYER:unmapped-snapshot")
end)
gnoblin.events.on("gnoblin.layer.removed", function(event)
    if event.last.namespace ~= namespace then return end
    assert(layer_from_snapshot(event.layer_id) == nil,
        "removed callback must see the layer absent from gnoblin.layers.list()")
    print("LUA_LAYER:removed-snapshot")
end)
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/layer-lifecycle-config-reload.txt"
timeout 15 "$GNOBLIN_LAYER_LIFECYCLE_CLIENT" \
    > "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt" 2>&1 &
layer_client_pid=$!
unmapped_seen=false
for _ in {1..100}; do
    if grep -Fxq 'CLIENT:unmapped' "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"; then
        unmapped_seen=true
        break
    fi
    if ! kill -0 "$layer_client_pid" 2>/dev/null; then break; fi
    sleep 0.1
done
if [[ "$unmapped_seen" != true ]]; then
    cat "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt" >&2
    echo 'Timed out waiting for the layer client to reach its unmap commit' >&2
    wait "$layer_client_pid" || true
    exit 1
fi
cat > "$XDG_RUNTIME_DIR/layer-live-snapshot.lua" <<'LUA'
local found = false
for _, layer in ipairs(gnoblin.layers.list()) do
    if layer.namespace == "gnoblin-lua-layer-lifecycle-e2e" then
        found = true
        print("LUA_LAYER:live-unmap:" .. tostring(layer.mapped))
    end
end
assert(found, "unmapped layer role must remain in the live snapshot until removal")
LUA
timeout 5 gnoblinctl lua "$XDG_RUNTIME_DIR/layer-live-snapshot.lua"
if ! wait "$layer_client_pid"; then
    cat "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt" >&2
    echo 'Layer lifecycle client failed or exceeded its 15 second timeout' >&2
    exit 1
fi
cat "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"
grep -Fxq 'CLIENT:role-created-unmapped' "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"
grep -Fxq 'CLIENT:mapped' "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"
grep -Fxq 'CLIENT:unmapped' "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"
grep -Fxq 'CLIENT:removed' "$XDG_RUNTIME_DIR/layer-lifecycle-client.txt"
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
import socket
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

event_name = "gnoblin.runtime.status-changed"
event_socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
event_socket.settimeout(20)
event_socket.connect(os.environ["GNOBLIN_COMPOSITOR_SOCKET"])
event_stream = event_socket.makefile("r", encoding="utf-8")
hello = json.loads(event_stream.readline())
assert hello.get("event") == "hello" and hello.get("api_minor", -1) >= 72, hello
assert event_name in hello.get("events", []), hello
event_socket.sendall((json.dumps({
    "op": "events",
    "api_version": {"major": 1, "minor": 72},
    "events": [event_name],
}) + "\n").encode())
subscription = json.loads(event_stream.readline())
assert subscription.get("event") == "subscribed", subscription
assert event_name in subscription.get("events", []), subscription

def expect_status_event(state, generation):
    event = json.loads(event_stream.readline())
    assert event.get("event") == event_name, event
    assert event.get("state") == state, event
    assert event.get("generation") == generation, event
    assert event.get("sequence", 0) > 0 and event.get("time", 0) > 0, event
    return event

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
expect_status_event("restarting", generation_before)

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
        expect_status_event("running", generation_before)
        print("WORKER:recovered-with-compositor-alive")
        break
    time.sleep(0.1)
else:
    raise AssertionError("Lua worker did not recover with the compositor alive")

config_path = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin" / "init.lua"
config_path.write_text(
    "gnoblin.configure { window_management = { focus_mode = 'click', "
    "focus_new_windows = 'strict' }, "
    "shortcuts = { shell_input_capture = { binding = 'Super', "
    "trigger = 'release', capture_input = true } }, "
    "input = { mouse = { drag_threshold = 37 } } }\n",
    encoding="utf-8",
)
subprocess.run([gnoblinctl, "config", "reload"], check=True, timeout=10)
deadline = time.monotonic() + 20
generation_after_reload = None
while time.monotonic() < deadline:
    status = runtime_status()
    if status and status.startswith("running:"):
        generation = int(status.split(":", 1)[1])
        if generation > generation_before:
            generation_after_reload = generation
            break
    time.sleep(0.05)
assert generation_after_reload is not None, "accepted config did not advance runtime generation"
expect_status_event("running", generation_after_reload)

worker_current, compositor_current = worker_and_compositor()
assert worker_current and compositor_current == compositor_before, (
    worker_current, compositor_current, compositor_before
)
for retry in range(5):
    os.kill(worker_current, signal.SIGKILL)
    terminal_failure = retry == 4
    terminal_state_observed = False
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        status = runtime_status()
        if status and status.startswith("restarting:"):
            assert int(status.split(":", 1)[1]) == generation_after_reload, status
            break
        if terminal_failure and status == f"unavailable:{generation_after_reload}":
            terminal_state_observed = True
            break
        time.sleep(0.01)
    else:
        raise AssertionError(f"runtime recovery {retry + 2} did not report restarting")
    expect_status_event("restarting", generation_after_reload)

    if retry < 4:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            worker_after, compositor_after = worker_and_compositor()
            status = runtime_status()
            if (
                worker_after
                and worker_after != worker_current
                and compositor_after == compositor_current
                and status == f"running:{generation_after_reload}"
            ):
                worker_current = worker_after
                expect_status_event("running", generation_after_reload)
                break
            time.sleep(0.1)
        else:
            raise AssertionError(f"runtime recovery {retry + 2} did not restart the worker")
    else:
        if not terminal_state_observed:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                status = runtime_status()
                if status == f"unavailable:{generation_after_reload}":
                    break
                time.sleep(0.05)
            else:
                raise AssertionError(
                    "runtime status did not become unavailable after retry exhaustion"
                )
        expect_status_event("unavailable", generation_after_reload)
        os.kill(compositor_current, 0)
        ping = subprocess.run(
            [gnoblinctl, "--timeout", "1", "ping"],
            check=True,
            capture_output=True,
            text=True,
            timeout=3,
        )
        assert ping.stdout.strip() == "pong", ping.stdout
        print("WORKER:retry-exhaustion-reported-unavailable")

event_socket.shutdown(socket.SHUT_RDWR)
event_stream.close()
event_socket.close()
print("RUNTIME_STATUS_EVENT:recovery-config-reload-and-terminal-failure")
PY
python3 "$GNOBLIN_UI_SESSION_TEST"
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
    GNOBLIN_UI_SESSION_TEST="$ROOT/tests/ui-session-devkit.py" \
    GNOBLIN_FOCUS_TEST_CLIENT="$fixture_root/focus-transfer-client" \
    GNOBLIN_FOCUS_TEST_SCRIPT="$ROOT/tests/test-focus-transfer.py" \
    GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT="$fixture_root/input-source-focus-client" \
    GNOBLIN_LAYER_LIFECYCLE_CLIENT="$fixture_root/layer-lifecycle-lua-client" \
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
require_output 'GSETTINGS:workspace-names-untouched'
require_output 'LUA_API:workspace-config'
require_output 'LUA_API:configured-shortcut-input-capture'
require_output 'LUA_API:input-accessibility-config'
require_output 'LUA_API:mouse-double-click-time'
require_output 'CONFIG:click'
require_output 'WINDOWS:json'
require_output 'WORKSPACE:next'
require_output 'WORKER:recovered-with-compositor-alive'
require_output 'RUNTIME_STATUS_EVENT:recovery-config-reload-and-terminal-failure'
require_output 'WORKER:retry-exhaustion-reported-unavailable'
require_output 'INPUT_SOURCE:empty-without-lua-setting'
require_output 'INPUT_SOURCE:configured-from-lua'
require_output 'INPUT_SOURCE:selected-through-cli'
require_output 'INPUT:mouse-drag-threshold-inherited'
require_output 'IBUS:selected-through-cli'
require_output 'INPUT_SOURCE:ibus-to-xkb'
require_output 'INPUT_SOURCE:xkb-to-ibus'
require_output 'IBUS:owner-lost'
require_output 'IBUS:reconnected-after-owner-restart'
require_output 'INPUT_SOURCE:cleared-with-lua-config'
require_output 'CONFIG_RELOAD:stable'
require_output 'XWAYLAND:xsettings-live-scaling-selection-loss'
require_output 'LUA_API:runtime-status'
require_output 'LUA_API:snapshots'
require_output 'UI_SESSION:capability-and-watch'
require_output 'UI_SESSION:state-update-and-late-snapshot'
require_output 'UI_SESSION:ownership-and-payload-validation'
require_output 'UI_SESSION:command-broadcast'
require_output 'UI_SESSION:disconnect-clears-state'
require_output 'PASS: native shell UI session relay works in a fresh Gnoblin devkit'
require_output 'CLI_MUTTER_EVENT:socket-forwarded'
require_output 'PASS: Gnoblin denied activation without user context and emitted the denial event'
require_output 'PASS: workspace animation rendered '
require_output 'PASS: resize animation rendered '
require_output 'LUA_LAYER:live-unmap:false'
for layer_event in created-snapshot mapped-snapshot unmapped-snapshot removed-snapshot; do
    if ! grep -Fq "LUA_LAYER:$layer_event" "$fixture_root/state/devkit-last.log"; then
        echo "Missing Lua layer lifecycle snapshot proof: $layer_event" >&2
        tail -n 80 "$fixture_root/state/devkit-last.log" >&2
        exit 1
    fi
done
printf '%s\n' 'PASS: Lua layer lifecycle callbacks see current layer snapshots'
if ! grep -Fq 'LUA_API:activity-event-snapshot' "$fixture_root/state/devkit-last.log"; then
    echo 'Missing Lua session activity event proof in the devkit runtime log' >&2
    tail -n 80 "$fixture_root/state/devkit-last.log" >&2
    exit 1
fi
for animation_event in gnoblin.animation.started gnoblin.animation.finished; do
    if ! grep -Fq "LUA_WORKSPACE_ANIMATION:$animation_event" "$fixture_root/state/devkit-last.log"; then
        echo "Missing Lua workspace animation event: $animation_event" >&2
        tail -n 80 "$fixture_root/state/devkit-last.log" >&2
        exit 1
    fi
done
grep -q 'restarting Lua runtime worker' "$fixture_root/state/devkit-last.log"
printf '%s\n' 'PASS: Lua config and native control API work in the supervised nested runtime'

per_window_output="$(GNOBLIN_DEVKIT_KEEP_SESSION=1 \
    GNOBLIN_TEST_IBUS_DAEMON=1 \
    GNOBLIN_STATE_DIR="$fixture_root/per-window-input-state" \
    XDG_DATA_DIRS="$fixture_root/data${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}:/usr/local/share:/usr/share" \
    GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
    GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
    GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
    GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT="$fixture_root/input-source-focus-client" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$ROOT/tests/fixtures/devkit-per-window-input" \
    GNOBLIN_DEVKIT_EXEC="bash $ROOT/tests/test-per-window-input-devkit.sh" \
    timeout 90 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$per_window_output" >&2
    exit 1
}
for expected in \
    'INPUT_SOURCE:per-window-config-enabled' \
    'INPUT_SOURCE:per-window-first-focus-inherits' \
    'INPUT_SOURCE:per-window-restores-A' \
    'INPUT_SOURCE:per-window-restores-B' \
    'INPUT_SOURCE:per-window-restores-XKB-from-IBus' \
    'INPUT_SOURCE:per-window-restores-IBus'; do
    if ! grep -Fq -- "$expected" <<<"$per_window_output"; then
        printf 'Missing expected per-window input output: %s\n' "$expected" >&2
        printf '%s\n' 'Captured per-window devkit output:' "$per_window_output" >&2
        exit 1
    fi
done
printf '%s\n' 'PASS: per-window input sources restore in a smart-focus DevKit session'

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
    'assert(s.session_state == "running" and type(s.session_revision) == "number"); '
    'print("SESSION_STATUS:"..s.state..":"..s.session_state..":"..s.session_revision)\n',
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

supervisor_current = supervisor_after
for retry in range(5):
    os.kill(supervisor_current, signal.SIGKILL)
    terminal_failure = retry == 4
    terminal_state_observed = False
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        status = runtime_status()
        if status and status.startswith("restarting:"):
            assert int(status.split(":", 1)[1]) == generation_before, status
            break
        if terminal_failure and status == f"unavailable:{generation_before}":
            terminal_state_observed = True
            break
        time.sleep(0.01)
    else:
        raise AssertionError(f"supervisor recovery {retry + 2} did not report restarting")

    if retry < 4:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            compositor_after, supervisor_after = processes()
            config_after = config_snapshot()
            status = runtime_status()
            if (
                supervisor_after
                and supervisor_after != supervisor_current
                and compositor_after == compositor_before
                and config_after == config_before
                and status == f"running:{generation_before}"
            ):
                supervisor_current = supervisor_after
                break
            time.sleep(0.1)
        else:
            raise AssertionError(f"supervisor recovery {retry + 2} did not restart")
    else:
        if not terminal_state_observed:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                status = runtime_status()
                if status == f"unavailable:{generation_before}":
                    break
                time.sleep(0.05)
            else:
                raise AssertionError(
                    "runtime status did not become unavailable after supervisor retry exhaustion"
                )
        os.kill(compositor_before, 0)
        ping = subprocess.run(
            [gnoblinctl, "--timeout", "1", "ping"],
            check=True,
            capture_output=True,
            text=True,
            timeout=3,
        )
        assert ping.stdout.strip() == "pong", ping.stdout
        print("SUPERVISOR:retry-exhaustion-reported-unavailable")
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
grep -q 'SUPERVISOR:retry-exhaustion-reported-unavailable' <<<"$guardian_output"
grep -q 'AUTOSTART:ran-once-across-supervisor-recovery' <<<"$guardian_output"
grep -q 'SESSION_STATUS:available-with-supervisor-stopped' <<<"$guardian_output"
printf '%s\n' 'PASS: session supervisor recovery and terminal status preserve the Mutter session'

startup_fixture="$fixture_root/startup-input-config"
mkdir -p "$startup_fixture/gnoblin"
cat >"$startup_fixture/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "gb"},
            {type = "xkb", id = "us"},
        },
    },
}
LUA
cat >"$fixture_root/check-startup-input.lua" <<'LUA'
local sources = gnoblin.input.sources()
assert(#sources == 2 and sources[1].id == "gb" and sources[2].id == "us")
local current = gnoblin.input.current_source()
assert(current and current.type == "xkb" and current.id == "gb")
print("INPUT_SOURCE:startup-keymap-gb")
LUA
startup_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
for _ in {1..100}; do
    if gnoblinctl lua "$GNOBLIN_STARTUP_INPUT_CHECK" \
        >"$XDG_RUNTIME_DIR/startup-input-check.txt" 2>&1; then
        cat "$XDG_RUNTIME_DIR/startup-input-check.txt"
        exit 0
    fi
    sleep 0.05
done
cat "$XDG_RUNTIME_DIR/startup-input-check.txt" >&2
echo 'Lua-configured startup keyboard layout did not become active' >&2
exit 1
SCRIPT
)
startup_output="$(
    GNOBLIN_STATE_DIR="$fixture_root/startup-input-state" \
        XDG_DATA_DIRS="$fixture_root/data${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}:/usr/local/share:/usr/share" \
        GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
        GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
        GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
        GNOBLIN_DEVKIT_CONFIG_SOURCE="$startup_fixture" \
        GNOBLIN_STARTUP_INPUT_CHECK="$fixture_root/check-startup-input.lua" \
        GNOBLIN_DEVKIT_EXEC="$startup_exec" \
        timeout 45 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1
)" || {
    printf '%s\n' "$startup_output" >&2
    exit 1
}
grep -q 'INPUT_SOURCE:startup-keymap-gb' <<<"$startup_output"
printf '%s\n' 'PASS: Lua input-source config initializes the startup keymap'

unsupported_action_fixture="$fixture_root/unsupported-action-config"
mkdir -p "$unsupported_action_fixture/gnoblin"
cat >"$unsupported_action_fixture/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    keybindings = {
        wm = {panel_run_dialog = {"<Super>r"}},
    },
}
LUA
if unsupported_action_output="$(
    GNOBLIN_STATE_DIR="$fixture_root/unsupported-action-state" \
        GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
        GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
        GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
        GNOBLIN_DEVKIT_CONFIG_SOURCE="$unsupported_action_fixture" \
        timeout 45 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1
)"; then
    echo 'Keybinding action without a Mutter handler unexpectedly started the Gnoblin session' >&2
    exit 1
fi
grep -q 'keybinding has no executable Mutter handler: wm.panel_run_dialog' \
    <<<"$unsupported_action_output"
printf '%s\n' 'PASS: startup rejects keybinding actions without a Mutter handler'

released_binding_fixture="$fixture_root/released-keybinding-config"
mkdir -p "$released_binding_fixture/gnoblin"
cat >"$released_binding_fixture/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    keybindings = {
        wm = {unmaximize = {}},
    },
    shortcuts = {
        restore_or_minimize = {
            binding = "<Super>Down",
            command = {"/usr/bin/true"},
        },
    },
}
LUA
released_binding_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
shortcuts="$(gnoblinctl shortcut list --json)"
grep -q 'restore_or_minimize' <<<"$shortcuts"
grep -q '<Super>Down' <<<"$shortcuts"
echo 'SHORTCUT:claimed-released-mutter-binding'
SCRIPT
)
released_binding_output="$(
    GNOBLIN_STATE_DIR="$fixture_root/released-keybinding-state" \
        GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
        GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
        GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
        GNOBLIN_DEVKIT_CONFIG_SOURCE="$released_binding_fixture" \
        GNOBLIN_DEVKIT_EXEC="$released_binding_exec" \
        timeout 45 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1
)" || {
    printf '%s\n' "$released_binding_output" >&2
    exit 1
}
grep -q 'SHORTCUT:claimed-released-mutter-binding' <<<"$released_binding_output"
printf '%s\n' 'PASS: Lua shortcut claims a Mutter binding released during startup'

effect_fixture="$fixture_root/effect-ownership-config"
mkdir -p "$effect_fixture/gnoblin"
cat >"$effect_fixture/gnoblin/init.lua" <<'LUA'
gnoblin.window_rule {
    match = {title = "^Gnoblin effect ownership fixture$"},
    corners = {
        radius = 14,
        mode = "force",
        shadow = {x = 0, y = 4, blur = 12, spread = 0, opacity = 0.4, color = "#000000"},
    },
    shader = "effect.frag",
    shader_uniforms = {strength = 0.5},
}
LUA
cat >"$effect_fixture/gnoblin/effect.frag" <<'GLSL'
uniform float strength;
vec4 gnoblin_effect(vec4 color, vec2 uv) {
    return vec4(mix(color.rgb, vec3(1.0, 0.0, 0.0), strength), color.a);
}
GLSL
effect_ownership_output="$(
    GNOBLIN_DEVKIT_KEEP_SESSION=1 \
        GNOBLIN_STATE_DIR="$fixture_root/effect-ownership-state" \
        XDG_DATA_DIRS="$fixture_root/data${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}:/usr/local/share:/usr/share" \
        GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
        GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
        GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
        GNOBLIN_DEVKIT_CONFIG_SOURCE="$effect_fixture" \
        GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT="$fixture_root/input-source-focus-client" \
        GNOBLIN_DEVKIT_EXEC="python3 '$ROOT/tests/test-window-effect-ownership.py'" \
        timeout 45 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1
)" || {
    printf '%s\n' "$effect_ownership_output" >&2
    exit 1
}
grep -q 'PASS: rounded clip, replacement shadow and shader survive map, repaint and close' \
    <<<"$effect_ownership_output"
printf '%s\n' "$effect_ownership_output"

lifecycle_output="$(
    GNOBLIN_DEVKIT_EXPECT_RUNTIME_EXIT=1 \
        GNOBLIN_STATE_DIR="$fixture_root/session-lifecycle-state" \
        GNOBLIN_PREFIX="$GNOBLIN_TEST_PREFIX" \
        GNOBLIN_RUNTIME_BIN="$GNOBLIN_TEST_RUNTIME" \
        GNOBLIN_DEVKIT_CTL="$GNOBLIN_TEST_PREFIX/bin/gnoblinctl" \
        GNOBLIN_DEVKIT_EXEC="python3 '$ROOT/tests/test-session-lifecycle.py'" \
        timeout 45 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1
)" || {
    printf '%s\n' "$lifecycle_output" >&2
    exit 1
}
grep -q 'SESSION_LIFECYCLE:stopping-socket-event' <<<"$lifecycle_output"
printf '%s\n' 'PASS: guardian publishes the stopping event before session shutdown'
