#!/usr/bin/env bash
# Verify Lua user scripts load, reload, and survive a rejected config reload.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/build/tmp"
fixture_root="$(mktemp -d "$ROOT/build/tmp/lua-scripting.XXXXXX")"
state_dir="$fixture_root/state"
config_dir="$fixture_root/config/gnoblin"
mkdir -p "$config_dir/scripts"
trap 'rm -rf -- "$fixture_root"' EXIT

cat >"$config_dir/init.lua" <<'LUA'
gnoblin.load("scripts/**/*.lua")
LUA

cat >"$config_dir/scripts/hello.lua" <<'LUA'
gnoblin.events.on("gnoblin.config.reloaded", function()
    print("LUA_SCRIPT A:reloaded")
end)
gnoblin.events.on("gnoblin.config.reload-failed", function()
    print("LUA_SCRIPT A:reload-failed")
end)
LUA

devkit_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
wait_for_runtime_log() {
    local marker="$1"
    for _ in $(seq 1 100); do
        grep -Fq -- "$marker" "$GNOBLIN_DEVKIT_RUNTIME_LOG" && return 0
        sleep 0.05
    done
    tail -n 80 "$GNOBLIN_DEVKIT_RUNTIME_LOG" >&2
    echo "FAIL: timed out waiting for Lua event $marker" >&2
    return 1
}

gnoblinctl config reload >/dev/null
wait_for_runtime_log 'LUA_SCRIPT A:reloaded'
gnoblinctl ping | grep -qx pong

cat >"$XDG_CONFIG_HOME/gnoblin/scripts/hello.lua" <<'LUA'
gnoblin.events.on("gnoblin.config.reloaded", function()
    print("LUA_SCRIPT B:reloaded")
end)
gnoblin.events.on("gnoblin.config.reload-failed", function()
    print("LUA_SCRIPT B:reload-failed")
end)
LUA
gnoblinctl config reload >/dev/null
wait_for_runtime_log 'LUA_SCRIPT B:reloaded'

cat >"$XDG_CONFIG_HOME/gnoblin/scripts/hello.lua" <<'LUA'
this is not valid Lua
LUA
if gnoblinctl config reload >"$XDG_RUNTIME_DIR/reload-error.txt" 2>&1; then
    echo 'FAIL: invalid Lua config reload succeeded' >&2
    exit 1
fi
grep -Eiq 'lua|syntax|unexpected|load' "$XDG_RUNTIME_DIR/reload-error.txt" || {
    cat "$XDG_RUNTIME_DIR/reload-error.txt" >&2
    echo 'FAIL: rejected reload did not report a Lua load error' >&2
    exit 1
}
wait_for_runtime_log 'LUA_SCRIPT B:reload-failed'
gnoblinctl ping | grep -qx pong
SCRIPT
)

output="$(GNOBLIN_STATE_DIR="$state_dir" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$fixture_root/config" \
    GNOBLIN_DEVKIT_EXEC="$devkit_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh" 2>&1)" || {
    printf '%s\n' "$output" >&2
    [[ ! -f "$state_dir/devkit-last.log" ]] || tail -n 80 "$state_dir/devkit-last.log" >&2
    exit 1
}

grep -q 'Gnoblin is ready on nested Wayland display' <<<"$output"
log="$state_dir/devkit-last.log"
if [[ -f "$log" ]]; then
    a_count="$(grep -c 'LUA_SCRIPT A:reloaded' "$log" || true)"
    b_count="$(grep -c 'LUA_SCRIPT B:reloaded' "$log" || true)"
    failed_count="$(grep -c 'LUA_SCRIPT B:reload-failed' "$log" || true)"
else
    a_count=0
    b_count=0
    failed_count=0
fi
if [[ "$a_count" -ne 1 || "$b_count" -ne 1 || "$failed_count" -ne 1 ]]; then
    [[ ! -f "$log" ]] || tail -n 80 "$log" >&2
    echo 'FAIL: Lua script reload or rollback did not preserve the expected handlers' >&2
    exit 1
fi

printf '%s\n' 'PASS: Lua user scripts reload and remain active after invalid config is rejected'
