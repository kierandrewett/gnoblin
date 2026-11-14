#!/usr/bin/env bash
# Takes pictures of the shipped JavaScript shell running on a real compositor.
#
# What runs is the release `gnoblin` binary as a session: the guardian starts the compositor (headless backend,
# software renderer), the services role and the runtime role, and the runtime role runs the stock JavaScript shell.
# The clients are real Wayland programs (foot) and the pictures are taken through the compositor's own screencopy
# path, on its privileged socket, with grim. Nothing is drawn by this script.
#
# Usage: scripts/js-shell-screenshots.sh [OUT_DIR [SCENARIO...]]
#
# SCENARIO is one or more of: one-window cascade master-stack master-stack-decorated dwindle dwindle-decorated
# shadows-software shadows-gles csd-double csd-own csd-tiled reload-tiled transition-mid window-menu lock. The default is all of them.
# GNOBLIN_BIN_DIR names where `gnoblin` and `gnoblinctl` are (default: $CARGO_TARGET_DIR/release, else
# target/release).
#
# Needs: grim, foot, dbus-daemon, a monospace font. `wtype` is needed only for the window-menu scenario.
# On Debian (an Agent37 instance has no GPU; the GLES scenario then runs on Mesa's llvmpipe):
#   apt-get install -y --no-install-recommends grim foot wtype dbus dbus-daemon fonts-dejavu-core \
#       libegl1 libgles2 libegl-mesa0 libgl1-mesa-dri
#   cargo build --release -p gnoblin -p gnoblinctl
#   scripts/js-shell-screenshots.sh ./shots
# design/js-runtime.md, "Live verification", says how to run it through a37-gate.sh.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${GNOBLIN_BIN_DIR:-${CARGO_TARGET_DIR:-$repo_root/target}/release}"
out_dir="${1:-$PWD/js-shell-shots}"
shift || true
wanted="${*:-one-window cascade master-stack master-stack-decorated dwindle dwindle-decorated shadows-software shadows-gles csd-double csd-own csd-tiled reload-tiled transition-mid window-menu lock}"

for tool in grim foot dbus-daemon; do
    command -v "$tool" >/dev/null || {
        echo "js-shell-screenshots: $tool is not installed" >&2
        exit 1
    }
done
for tool in gnoblin gnoblinctl; do
    [ -x "$bin_dir/$tool" ] || {
        echo "js-shell-screenshots: $bin_dir/$tool is not built" >&2
        exit 1
    }
done
mkdir -p "$out_dir"

work=""
session_pid=""
bus_pid=""
client_pids=()

# The session's own directories live under one temporary directory, so nothing of the machine's is read or written.
run_dir=""
config_dir=""
session_home=""
sessions=0

# kill_session: the guardian leads a process group of its own (setsid), so its children go with it. Returns when the
# group is gone, so a socket the old compositor unlinks late cannot be the new one's.
kill_session() {
    [ -n "$session_pid" ] || return 0
    kill -TERM -- "-$session_pid" 2>/dev/null || true
    wait "$session_pid" 2>/dev/null || true
    local tries=0
    while kill -0 -- "-$session_pid" 2>/dev/null && [ "$tries" -lt 100 ]; do
        sleep 0.1
        tries=$((tries + 1))
    done
    kill -KILL -- "-$session_pid" 2>/dev/null || true
    session_pid=""
    rm -f "$run_dir"/*.sock
}

stop_session() {
    local pid
    for pid in "${client_pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    client_pids=()
    kill_session
    if [ -n "$bus_pid" ]; then
        kill "$bus_pid" 2>/dev/null || true
        wait "$bus_pid" 2>/dev/null || true
        bus_pid=""
    fi
}

cleanup() {
    stop_session
    [ -z "$work" ] || rm -rf "$work"
}
trap cleanup EXIT

# start_session [INIT_TS_SOURCE]: boots the guardian over a fresh profile. With no source the config directory
# is empty, which is the zero-config session.
start_session() {
    work="${work:-$(mktemp -d)}"
    # A directory of its own per session: the compositor of the one before may still be unlinking its sockets.
    sessions=$((sessions + 1))
    session_home="$work/session$sessions"
    run_dir="$session_home/run"
    config_dir="$session_home/config/gnoblin"
    mkdir -p "$run_dir" "$config_dir" "$session_home/state" "$session_home/data" "$session_home/cache"
    chmod 700 "$run_dir"
    if [ $# -gt 0 ]; then
        printf '%s\n' "$1" >"$config_dir/init.ts"
    fi
    dbus-daemon --session --nofork --print-address >"$session_home/bus-address" 2>"$session_home/bus.log" &
    bus_pid=$!
    until grep -q '^unix:' "$session_home/bus-address" 2>/dev/null; do sleep 0.1; done
    boot_guardian
}

# boot_guardian: starts the session in a process group of its own and waits until the shell has committed a config.
boot_guardian() {
    : >"$session_home/session.log"
    setsid env -u WAYLAND_DISPLAY -u DISPLAY -u GNOBLIN_JS_STOCK_DIR -u GNOBLIN_CONFIG \
        XDG_RUNTIME_DIR="$run_dir" \
        XDG_CONFIG_HOME="$session_home/config" \
        XDG_STATE_HOME="$session_home/state" \
        XDG_DATA_HOME="$session_home/data" \
        XDG_CACHE_HOME="$session_home/cache" \
        DBUS_SESSION_BUS_ADDRESS="$(cat "$session_home/bus-address")" \
        GNOBLIN_IPC_SOCKET="$run_dir/ipc.sock" \
        "$bin_dir/gnoblin" >>"$session_home/session.log" 2>&1 &
    session_pid=$!
    local waited=0
    until grep -q 'configuration revision 1 accepted' "$session_home/session.log" 2>/dev/null; do
        sleep 0.2
        waited=$((waited + 1))
        if [ "$waited" -gt 600 ]; then
            echo "js-shell-screenshots: the session never accepted its configuration; its log:" >&2
            cat "$session_home/session.log" >&2
            exit 1
        fi
    done
}

# restart_session: ends the guardian and everything it started, then boots it again over the same directories. A
# setting the compositor reads only when it starts (compositor.renderer) takes effect from the config the first run
# committed.
restart_session() {
    kill_session
    boot_guardian
}

ctl() {
    GNOBLIN_IPC_SOCKET="$run_dir/ipc.sock" XDG_RUNTIME_DIR="$run_dir" "$bin_dir/gnoblinctl" "$@"
}

# window APP_ID TITLE BACKGROUND_HEX COMMAND [FOOT_OPTION...]: one foot window running COMMAND, in a tint of its own so the
# windows can be told apart. foot takes a colour without the leading `#`. COMMAND is what the picture shows, so it is a
# real program of the session (`gnoblinctl ...`) and not text typed for the picture. The compositor asks clients for
# client-side decorations while no shell draws them, so foot draws a title bar of its own then; foot as it comes asks for
# server-side (it does not need `-o csd.preferred=server`), and the shell's titlebar is its one bar whenever decorations
# are on. `-o csd.preferred=client` makes foot draw its own bar whatever the answer, and `none` leaves it no bar at all,
# which the compositor reads as a client that draws its own: the shell gives such a window none.
window() {
    local app_id="$1" title="$2" colour="$3" command="$4"
    shift 4
    WAYLAND_DISPLAY="$run_dir/gnoblin-wayland.sock" XDG_RUNTIME_DIR="$run_dir" LC_ALL=C.UTF-8 \
        GNOBLIN_IPC_SOCKET="$run_dir/ipc.sock" PATH="$bin_dir:$PATH" \
        foot --app-id "$app_id" --title "$title" \
        -o "colors.background=${colour#\#}" -o "colors.alpha=1" -o "main.pad=8x8" "$@" \
        sh -c "$command; exec sleep 3600" >/dev/null 2>&1 &
    client_pids+=("$!")
}

# plain_window APP_ID TITLE BACKGROUND_HEX COMMAND: foot as it comes, which asks for the shell's titlebar.
plain_window() {
    window "$@"
}

# shoot NAME: one PNG of the output through the privileged screencopy socket, beside what the compositor holds for
# its windows and the session's log, so a picture that looks wrong has its evidence next to it.
shoot() {
    sleep 4
    ctl --json window list >"$out_dir/$1.windows.json" 2>&1 || true
    ctl console --eval 'gnoblin.clients.map((c) => ({ title: c.title, x: c.x, y: c.y, width: c.width, height: c.height, inset: c.frameInset, tiled: c.tiled, workspace: c.workspace && c.workspace.id }))' \
        >"$out_dir/$1.clients.json" 2>&1 || true
    cp "$session_home/session.log" "$out_dir/$1.log"
    WAYLAND_DISPLAY="$run_dir/gnoblin-wayland-privileged.sock" XDG_RUNTIME_DIR="$run_dir" \
        grim "$out_dir/$1.png"
    echo "js-shell-screenshots: wrote $out_dir/$1.png"
}

# wait_windows COUNT: until the compositor holds COUNT windows, so the next client maps after the last was placed.
wait_windows() {
    local tries=0
    until [ "$(ctl --json window list | grep -o '"type":"gnoblin.window"' | wc -l)" -ge "$1" ]; do
        sleep 0.2
        tries=$((tries + 1))
        if [ "$tries" -gt 150 ]; then
            echo "js-shell-screenshots: $1 windows never mapped; the log:" >&2
            cat "$session_home/session.log" >&2
            exit 1
        fi
    done
    sleep 1
}

# three NAME: three windows, one after another so each is placed before the next maps.
three() {
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    plain_window org.example.windows "gnoblinctl window list" "#5f1e3a" "gnoblinctl window list"
    wait_windows 2
    plain_window org.example.version "gnoblinctl version" "#3a5f1e" "gnoblinctl version"
    wait_windows 3
}

# set_config SOURCE: writes init.ts into the running session's config directory and reloads it.
set_config() {
    printf '%s\n' "$1" >"$config_dir/init.ts"
    ctl config reload
    sleep 1
}

scenario_one_window() {
    start_session
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    shoot one-window
    stop_session
}

scenario_cascade() {
    start_session
    three
    shoot cascade
    stop_session
}

# tiled NAME DECORATIONS LAYOUT: three windows, then a reload to one workspace with LAYOUT. DECORATIONS is `true`,
# `false` or empty, which leaves the key out as the owner's example does.
tiled() {
    local decorations=""
    if [ -n "$2" ]; then
        decorations="decorations: $2,"
    fi
    start_session
    three
    set_config "import { DwindleLayout, MasterStackLayout, defineConfig } from \"gnoblin\";

export default defineConfig({
    $decorations
    workspaces: [{ id: \"dev\", layout: new $3 }],
});"
    shoot "$1"
    stop_session
}

scenario_master_stack() {
    tiled master-stack "" "MasterStackLayout({ ratio: 0.6, gap: 12 })"
}

scenario_master_stack_decorated() {
    tiled master-stack-decorated true "MasterStackLayout({ ratio: 0.6, gap: 12 })"
}

scenario_dwindle() {
    tiled dwindle "" "DwindleLayout({ gap: 12 })"
}

scenario_dwindle_decorated() {
    tiled dwindle-decorated true "DwindleLayout({ gap: 12 })"
}

# shadows NAME RENDERER: the stock config with a style that gives every window a shadow, on the named renderer. The
# renderer is chosen when the session starts, so the config is there from the first frame.
shadows() {
    start_session "import { defaultConfig, defineConfig } from \"gnoblin\";

export default defineConfig({
    ...defaultConfig,
    compositor: { renderer: \"$2\" },
    style: { shadows: [{ x: 0, y: 12, blur: 32, color: \"#000000cc\" }] },
});"
    restart_session
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    plain_window org.example.windows "gnoblinctl window list" "#5f1e3a" "gnoblinctl window list"
    wait_windows 2
    shoot "$1"
    stop_session
}

scenario_shadows_software() {
    shadows shadows-software software
}

scenario_shadows_gles() {
    shadows shadows-gles gles
}

# csd-double: foot as it comes, under the stock shell with its titlebar. Foot asks for server-side decorations, the shell
# has declared that it draws them, so foot draws none of its own and the picture shows exactly one title bar. Before
# the compositor negotiated this it answered client-side to every client and the picture showed two.
scenario_csd_double() {
    start_session
    window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    shoot csd-double
    stop_session
}

# csd-own: the same foot under a shell with decorations off. The compositor tells foot to draw its own title bar, so the
# picture shows foot's one bar and none of the shell's: a window is never left with no bar.
scenario_csd_own() {
    start_session
    set_config "import { defineConfig } from \"gnoblin\";

export default defineConfig({ decorations: false });"
    window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    shoot csd-own
    stop_session
}

# csd-tiled: the same client in a tiled workspace with the shell's titlebars: one bar on each tile.
scenario_csd_tiled() {
    start_session
    set_config "import { DwindleLayout, defineConfig } from \"gnoblin\";

export default defineConfig({
    decorations: true,
    workspaces: [{ id: \"dev\", layout: new DwindleLayout({ gap: 12 }) }],
});"
    window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    shoot csd-tiled
    stop_session
}

# window-menu: the stock window menu, opened by the window-menu key (Alt+Space) that wtype sends through the virtual
# keyboard protocol. A titlebar right click needs a pointer, and a shipped session offers no way to inject one: its EIS
# socket stays closed until a remote desktop session is approved, so the click is proved by the real-compositor tests
# (crates/gnoblin/tests/js_shell.rs) and not drawn here.
scenario_window_menu() {
    start_session
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    WAYLAND_DISPLAY="$run_dir/gnoblin-wayland-privileged.sock" XDG_RUNTIME_DIR="$run_dir" wtype -M alt -k space -m alt \
        >"$session_home/wtype.log" 2>&1 || true
    shoot window-menu
    cp "$session_home/wtype.log" "$out_dir/window-menu.wtype.log"
    stop_session
}

# lock: the session locked, then asked for a picture. The compositor refuses every capture of a locked session (a still, a
# stream and screencopy alike), so no PNG is the expected outcome and what is kept is grim's answer. The lock screen and
# the compositor's lock card are therefore checked by the scene (crates/gnoblin/tests/js_lock.rs) and not drawn here.
scenario_lock() {
    start_session
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    ctl lock
    sleep 3
    if WAYLAND_DISPLAY="$run_dir/gnoblin-wayland-privileged.sock" XDG_RUNTIME_DIR="$run_dir" \
        grim "$out_dir/lock.png" >"$out_dir/lock.grim.log" 2>&1; then
        echo "js-shell-screenshots: wrote $out_dir/lock.png"
    else
        echo "js-shell-screenshots: a locked session refused the capture: $(cat "$out_dir/lock.grim.log")"
    fi
    cp "$session_home/session.log" "$out_dir/lock.log"
    stop_session
}

# reload-tiled: three tiled windows, then a reload of the same file. The shell's new generation takes the windows over, so
# the picture after must be the picture before, pixel for pixel.
scenario_reload_tiled() {
    start_session
    three
    set_config "import { MasterStackLayout, defineConfig } from \"gnoblin\";

export default defineConfig({
    decorations: true,
    workspaces: [{ id: \"dev\", layout: new MasterStackLayout({ ratio: 0.6, gap: 12 }) }],
});"
    shoot reload-before
    ctl config reload
    sleep 2
    shoot reload-after
    if cmp -s "$out_dir/reload-before.png" "$out_dir/reload-after.png"; then
        echo "js-shell-screenshots: the pictures before and after the reload are identical"
    else
        echo "js-shell-screenshots: the pictures before and after the reload DIFFER"
    fi
    stop_session
}

# transition-mid: one window moved 600 px right in a requestTransition of 6 s, and a picture 2 s into it. The transition
# runs on the compositor's real clock, so a long duration is how a script catches a frame in the middle of it.
scenario_transition_mid() {
    start_session
    plain_window org.example.status "gnoblinctl status" "#1e3a5f" "gnoblinctl status"
    wait_windows 1
    ctl console --eval 'gnoblin.requestTransition(() => { gnoblin.clients.at(0).x = 600; }, { duration: 6000, easing: "linear" })' >"$out_dir/transition.eval.log" 2>&1 || true
    sleep 2
    WAYLAND_DISPLAY="$run_dir/gnoblin-wayland-privileged.sock" XDG_RUNTIME_DIR="$run_dir" grim "$out_dir/transition-mid.png"
    echo "js-shell-screenshots: wrote $out_dir/transition-mid.png"
    shoot transition-end
    stop_session
}

main() {
    local scenario
    for scenario in $wanted; do
        case "$scenario" in
            one-window | cascade | master-stack | master-stack-decorated | dwindle | dwindle-decorated | shadows-software | shadows-gles | csd-double | csd-own | csd-tiled | reload-tiled | transition-mid | window-menu | lock)
                "scenario_${scenario//-/_}"
                ;;
            *)
                echo "js-shell-screenshots: unknown scenario $scenario" >&2
                exit 2
                ;;
        esac
    done
}

# Sourced by a caller that wants the helpers only, it runs nothing.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main
fi
