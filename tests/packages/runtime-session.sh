#!/usr/bin/env bash
# The session cases of the runtime layer (R1 to R7). tests/packages/runtime.sh runs this as the test user in a D-Bus
# session, with XDG_RUNTIME_DIR, XDG_SESSION_TYPE=wayland and XDG_SESSION_ID set, as a login would.
#
# usage: runtime-session.sh OUT_DIR
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
out_dir="$1"
LAYER=runtime
# shellcheck source=tests/packages/lib.sh
. "$here/lib.sh"
RESULTS="$out_dir/results.tsv"
CASE_LOG_DIR="$out_dir/cases"
probe=(python3 "$here/runtime_probe.py")
ctl=/usr/bin/gnoblinctl
export WAYLAND_DISPLAY=wayland-0
session_log="$out_dir/session.log"
guardian_pid=""

stop_session() {
    if [ -n "$guardian_pid" ] && kill -0 "$guardian_pid" 2>/dev/null; then
        kill -TERM "$guardian_pid" 2>/dev/null || true
        sleep 5
        kill -KILL "$guardian_pid" 2>/dev/null || true
    fi
}
trap stop_session EXIT

# R1: the session starts and stays up: the Wayland socket and the compositor bridge appear within 60 seconds.
case_start() {
    for _ in $(seq 1 600); do
        if [ -S "$XDG_RUNTIME_DIR/wayland-0" ] && [ -S "$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock" ]; then
            break
        fi
        kill -0 "$guardian_pid" || {
            echo "the session exited:"
            tail -n 40 "$session_log"
            return 1
        }
        sleep 0.1
    done
    test -S "$XDG_RUNTIME_DIR/wayland-0"
    test -S "$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock"
    for _ in $(seq 1 100); do
        "$ctl" --timeout 1 ping >/dev/null 2>&1 && break
        sleep 0.2
    done
    "$ctl" --timeout 2 ping
    sleep 2
    kill -0 "$guardian_pid"
}

# R2: every global that the build holds is advertised.
case_globals() {
    wayland-info >"$out_dir/wayland-info.txt"
    "${probe[@]}" globals "$out_dir/wayland-info.txt"
}

# R3: the compositor bridge greets, answers a ping and a monitor snapshot.
case_bridge() {
    "${probe[@]}" bridge
}

# R4: gnoblinctl reaches the session.
case_gnoblinctl() {
    "$ctl" --timeout 5 ping
    "$ctl" --timeout 5 version | tee "$out_dir/version.json"
    grep -q '"gnoblin"' "$out_dir/version.json"
    "$ctl" --timeout 5 status
    "$ctl" --timeout 5 output list --json | tee "$out_dir/outputs.json"
    grep -q '1280' "$out_dir/outputs.json"
}

# R5: the default configuration loads, a broken one is refused, and the session keeps running.
case_config() {
    "$ctl" init
    local config
    config="$("$ctl" config path)"
    test -s "$config"
    "$ctl" --timeout 20 config reload
    cp "$config" "$out_dir/init.lua.good"
    printf '\nthis is not Lua (\n' >>"$config"
    if "$ctl" --timeout 20 config reload; then
        echo "a configuration with a Lua syntax error was accepted"
        return 1
    fi
    "$ctl" --timeout 5 ping
    cp "$out_dir/init.lua.good" "$config"
    "$ctl" --timeout 20 config reload
}

# R6: screencopy captures the output, and shows a layer-shell client that fills it with one colour.
case_screenshot() {
    grim -t ppm "$out_dir/screenshot-before.ppm"
    "${probe[@]}" image "$out_dir/screenshot-before.ppm" 1280 720
    # The # form: swaybg 1.2.0 (Ubuntu 24.04) refuses a colour without it, and leaves the output black.
    swaybg -c '#ff00ff' >"$out_dir/swaybg.log" 2>&1 &
    local swaybg_pid=$!
    local ok=1
    for _ in $(seq 1 20); do
        sleep 0.5
        grim -t ppm "$out_dir/screenshot.ppm"
        if "${probe[@]}" image "$out_dir/screenshot.ppm" 1280 720 ff00ff; then
            ok=0
            break
        fi
    done
    kill "$swaybg_pid" 2>/dev/null || true
    wait "$swaybg_pid" 2>/dev/null || true
    return "$ok"
}

# R7: the session stops cleanly on gnoblinctl logout, and the logs hold no unexpected error.
case_shutdown() {
    "$ctl" --timeout 10 logout
    for _ in $(seq 1 300); do
        [ -s "$out_dir/session.status" ] && break
        sleep 0.1
    done
    if [ ! -s "$out_dir/session.status" ]; then
        echo "the session is still running 30 seconds after the logout"
        return 1
    fi
    echo "the session exited with status $(cat "$out_dir/session.status")"
    [ "$(cat "$out_dir/session.status")" = 0 ]
    "${probe[@]}" logs "$session_log" "$HOME/.local/state/gnoblin/"*.log
}

# The guardian, as a login starts it. Its exit status goes to a file, because the cases run in subshells.
(
    # A login starts the session with no display variables.
    env -u WAYLAND_DISPLAY /usr/lib/gnoblin/bin/gnoblin.real --no-xwayland >"$session_log" 2>&1 &
    echo "$!" >"$out_dir/session.pid"
    status=0
    wait "$!" || status=$?
    echo "$status" >"$out_dir/session.status"
) &
for _ in $(seq 1 50); do
    [ -s "$out_dir/session.pid" ] && break
    sleep 0.1
done
guardian_pid="$(cat "$out_dir/session.pid")"
run_case R1 "the session starts" fatal case_start
run_case R2 "Wayland globals" continue case_globals
run_case R3 "compositor bridge" continue case_bridge
run_case R4 "gnoblinctl" continue case_gnoblinctl
run_case R5 "Lua configuration loads and validates" continue case_config
run_case R6 "screenshot through screencopy" continue case_screenshot
run_case R7 "clean shutdown and logs" continue case_shutdown
