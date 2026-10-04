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
gnoblinctl input select xkb us > "$XDG_RUNTIME_DIR/xkb-source-after-ibus.txt"
cat > "$XDG_RUNTIME_DIR/xkb-current.lua" <<'LUA'
local current = gnoblin.input.current_source()
assert(current and current.type == "xkb" and current.id == "us")
print("INPUT_SOURCE:ibus-to-xkb")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/xkb-current.lua"
select_ibus_source "$XDG_RUNTIME_DIR/ibus-source-reselected.txt"
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
select_ibus_source "$XDG_RUNTIME_DIR/ibus-source-reselect.txt"
gnoblinctl lua "$XDG_RUNTIME_DIR/ibus-current.lua"
printf 'IBUS:reconnected-after-owner-restart\n'
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
    },
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
            {type = "ibus", id = "xkb:us::eng"},
        },
        per_window = true,
    },
}
LUA
gnoblinctl config reload > "$XDG_RUNTIME_DIR/per-window-input-config.txt"
cat > "$XDG_RUNTIME_DIR/per-window-enabled.lua" <<'LUA'
assert(gnoblin.settings.input_sources.per_window == true)
print("INPUT_SOURCE:per-window-config-enabled")
LUA
gnoblinctl lua "$XDG_RUNTIME_DIR/per-window-enabled.lua"
(
    title_a='Gnoblin per-window input A'
    title_b='Gnoblin per-window input B'
    window_log_a="$XDG_RUNTIME_DIR/per-window-window-a.log"
    window_log_b="$XDG_RUNTIME_DIR/per-window-window-b.log"
    windows_json="$XDG_RUNTIME_DIR/per-window-windows.json"
    input_json="$XDG_RUNTIME_DIR/per-window-input.json"
    focus_client="$GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT"
    "$focus_client" window "$title_a" > "$window_log_a" 2>&1 &
    window_a_pid=$!
    "$focus_client" window "$title_b" > "$window_log_b" 2>&1 &
    window_b_pid=$!
    cleanup_windows() {
        local result=$?
        kill -TERM "$window_a_pid" "$window_b_pid" 2>/dev/null || true
        wait "$window_a_pid" 2>/dev/null || true
        wait "$window_b_pid" 2>/dev/null || true
        if ((result != 0)); then
            printf '%s\n' 'Input-source fixture A log:' >&2
            cat "$window_log_a" >&2
            printf '%s\n' 'Input-source fixture B log:' >&2
            cat "$window_log_b" >&2
        fi
        return "$result"
    }
    trap cleanup_windows EXIT
    window_matches() {
        local title="$1"
        local focused="$2"
        gnoblinctl --json window list > "$windows_json" || return 1
        python3 - "$windows_json" "$title" "$focused" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    value = json.load(stream)
windows = value if isinstance(value, list) else value.get("windows", [])
expected_focus = None if sys.argv[3] == "any" else sys.argv[3] == "true"
for window in windows:
    if window.get("title") == sys.argv[2] and (
        expected_focus is None or bool(window.get("focused")) == expected_focus
    ):
        sys.exit(0)
sys.exit(1)
PY
    }
    wait_for_window() {
        local title="$1"
        for _ in {1..100}; do
            if window_matches "$title" any; then
                return 0
            fi
            sleep 0.05
        done
        cat "$windows_json" >&2
        echo "test window did not appear: $title" >&2
        return 1
    }
    wait_for_focus() {
        local title="$1"
        for _ in {1..100}; do
            if window_matches "$title" true; then
                return 0
            fi
            sleep 0.05
        done
        cat "$windows_json" >&2
        echo "test window did not receive focus: $title" >&2
        return 1
    }
    wait_for_source() {
        local source_type="$1"
        local source_id="$2"
        for _ in {1..100}; do
            gnoblinctl --json input current > "$input_json" 2>/dev/null || true
            if python3 - "$input_json" "$source_type" "$source_id" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as stream:
        current = json.load(stream)
except (OSError, json.JSONDecodeError):
    sys.exit(1)
source = current.get("source")
if current.get("available") and source and source.get("type") == sys.argv[2] \
        and source.get("id") == sys.argv[3]:
    sys.exit(0)
sys.exit(1)
PY
            then
                return 0
            fi
            sleep 0.05
        done
        cat "$input_json" >&2
        echo "input source did not become active: $source_id" >&2
        return 1
    }
    select_source() {
        local source_id="$1"
        local result_file="$XDG_RUNTIME_DIR/select-$source_id.txt"
        for _ in {1..100}; do
            if gnoblinctl --timeout 2 input select xkb "$source_id" > "$result_file" 2>&1; then
                return 0
            fi
            sleep 0.05
        done
        cat "$result_file" >&2
        echo "could not select XKB source: $source_id" >&2
        return 1
    }
    activate_window() {
        local title="$1"
        timeout 7 "$focus_client" activate "$title"
        wait_for_focus "$title"
    }

    wait_for_window "$title_a"
    wait_for_window "$title_b"
    activate_window "$title_a"
    select_source us
    wait_for_source xkb us
    activate_window "$title_b"
    wait_for_source xkb us
    printf 'INPUT_SOURCE:per-window-first-focus-inherits\n'
    select_source gb
    wait_for_source xkb gb
    activate_window "$title_a"
    wait_for_source xkb us
    printf 'INPUT_SOURCE:per-window-restores-A\n'
    activate_window "$title_b"
    wait_for_source xkb gb
    printf 'INPUT_SOURCE:per-window-restores-B\n'
    select_ibus_source "$XDG_RUNTIME_DIR/per-window-ibus-select.txt"
    wait_for_source ibus xkb:us::eng
    activate_window "$title_a"
    wait_for_source xkb us
    printf 'INPUT_SOURCE:per-window-restores-XKB-from-IBus\n'
    activate_window "$title_b"
    wait_for_source ibus xkb:us::eng
    printf 'INPUT_SOURCE:per-window-restores-IBus\n'
)
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
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
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
cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    window_management = {
        focus_mode = "click",
        focus_new_windows = "strict",
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
    "focus_new_windows = 'strict' }, input = { mouse = { drag_threshold = 37 } } }\n",
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
require_output 'INPUT_SOURCE:per-window-config-enabled'
require_output 'INPUT_SOURCE:per-window-first-focus-inherits'
require_output 'INPUT_SOURCE:per-window-restores-A'
require_output 'INPUT_SOURCE:per-window-restores-B'
require_output 'INPUT_SOURCE:per-window-restores-XKB-from-IBus'
require_output 'INPUT_SOURCE:per-window-restores-IBus'
require_output 'IBUS:owner-lost'
require_output 'IBUS:reconnected-after-owner-restart'
require_output 'INPUT_SOURCE:cleared-with-lua-config'
require_output 'CONFIG_RELOAD:stable'
require_output 'XWAYLAND:xsettings-live-scaling-selection-loss'
require_output 'LUA_API:runtime-status'
require_output 'LUA_API:snapshots'
require_output 'PASS: Gnoblin denied activation without user context and emitted the denial event'
require_output 'PASS: workspace animation lifecycle events reach socket clients'
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
