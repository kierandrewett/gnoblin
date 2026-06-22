#!/usr/bin/env bash
# Legacy smoke — run GNOME Shell as an isolated Wayland compositor, using the
# patched mutter build when MUTTER_BUILD_DIR is present, and prove layer-shell
# clients can bind and map against that compositor. gnoblin's real integration
# suite is tests/layer-shell via gnoblin-shell; this remains a reference check.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "test-nested: needs to run inside a Wayland session (no WAYLAND_DISPLAY)" >&2
    exit 1
fi

if [ -z "${GNOME_SHELL_BIN:-}" ] && [ -x "$REPO_ROOT/build/gnome-shell/src/gnome-shell" ]; then
    GNOME_SHELL_BIN="$REPO_ROOT/build/gnome-shell/src/gnome-shell"
else
    GNOME_SHELL_BIN="${GNOME_SHELL_BIN:-gnome-shell}"
fi
command -v "$GNOME_SHELL_BIN" >/dev/null || { echo "gnome-shell not installed" >&2; exit 1; }

OUT="${1:-/tmp/gnoblin-nested}"
MUTTER_BUILD_DIR="${MUTTER_BUILD_DIR:-/tmp/mutterbuild}"
GNOBLIN_PREFIX="${GNOBLIN_PREFIX:-$REPO_ROOT/install}"
VISIBLE="${VISIBLE:-0}"
RUN_CLIENTS="${RUN_CLIENTS:-1}"
RUN_GNOBLIN_CLIENTS="${RUN_GNOBLIN_CLIENTS:-0}"
RUN_PROBE="${RUN_PROBE:-1}"
CLIENT_TIMEOUT="${CLIENT_TIMEOUT:-30}"
AUTOSTART_TIMEOUT="${AUTOSTART_TIMEOUT:-120}"
HOLD_OPEN_SECONDS="${HOLD_OPEN_SECONDS:-0}"
AUTOSTART_GNOBLIN="${AUTOSTART_GNOBLIN:-0}"
TOPBAR_BIN="$GNOBLIN_PREFIX/bin/gnoblin-topbar"
DOCK_BIN="$GNOBLIN_PREFIX/bin/gnoblin-dock"
mkdir -p "$OUT"
rm -f "$OUT"/*.log "$OUT"/nested.png "$OUT"/waybar.json "$OUT"/waybar.css
rm -rf "$OUT/probe-build" "$OUT/layer-shell-probe" "$OUT/autostart"
mkdir -p "$OUT/xdg-data" "$OUT/xdg-config" "$OUT/xdg-cache"
export XDG_DATA_HOME="$OUT/xdg-data"
export XDG_CONFIG_HOME="$OUT/xdg-config"
export XDG_CACHE_HOME="$OUT/xdg-cache"

SHELL_LOG="$OUT/shell.log"
LOADER_LOG="$OUT/loader.log"

rc=0
ok() { echo "   ok  $*"; }
fail() { echo "   FAIL  $*"; rc=1; }

SHELL_TYPELIB_PATHS=()
for dir in \
    "$REPO_ROOT/build/gnome-shell/src" \
    "$REPO_ROOT/build/gnome-shell/src/st" \
    "$REPO_ROOT/build/gnome-shell/subprojects/gvc" \
    "$REPO_ROOT/build/gnome-shell/subprojects/libshew/src"
do
    [ -d "$dir" ] && SHELL_TYPELIB_PATHS+=("$dir")
done
if [ "${#SHELL_TYPELIB_PATHS[@]}" -gt 0 ]; then
    SHELL_TYPELIB_PATH="$(IFS=:; echo "${SHELL_TYPELIB_PATHS[*]}")"
    export GI_TYPELIB_PATH="$SHELL_TYPELIB_PATH${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
    export LD_LIBRARY_PATH="$SHELL_TYPELIB_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
if [ -f "$REPO_ROOT/build/gnome-shell/data/gnome-shell-dbus-interfaces.gresource" ]; then
    export GNOME_SHELL_DATADIR="$REPO_ROOT/build/gnome-shell/data"
fi
if [ -f "$REPO_ROOT/build/gnome-shell/data/gschemas.compiled" ]; then
    export GSETTINGS_SCHEMA_DIR="$REPO_ROOT/build/gnome-shell/data"
fi
if command -v gsettings >/dev/null 2>&1 &&
   command -v dbus-run-session >/dev/null 2>&1; then
    dbus-run-session -- \
        gsettings set org.gnome.shell welcome-dialog-last-shown-version '9999' \
        >/dev/null 2>&1 || true
fi

SHELL_ARGS=()
if "$GNOME_SHELL_BIN" --help 2>&1 | grep -q -- '--disable-extensions'; then
    SHELL_ARGS+=(--disable-extensions)
fi

case "$VISIBLE" in
    1|true|TRUE|yes|YES) VISIBLE=1 ;;
    *) VISIBLE=0 ;;
esac
case "$RUN_CLIENTS" in
    0|false|FALSE|no|NO) RUN_CLIENTS=0 ;;
    *) RUN_CLIENTS=1 ;;
esac
case "$RUN_GNOBLIN_CLIENTS" in
    1|true|TRUE|yes|YES) RUN_GNOBLIN_CLIENTS=1 ;;
    *) RUN_GNOBLIN_CLIENTS=0 ;;
esac
case "$RUN_PROBE" in
    0|false|FALSE|no|NO) RUN_PROBE=0 ;;
    *) RUN_PROBE=1 ;;
esac
case "$CLIENT_TIMEOUT" in
    ''|*[!0-9]*) CLIENT_TIMEOUT=30 ;;
esac
case "$AUTOSTART_TIMEOUT" in
    ''|*[!0-9]*) AUTOSTART_TIMEOUT=120 ;;
esac
case "$HOLD_OPEN_SECONDS" in
    ''|*[!0-9]*) HOLD_OPEN_SECONDS=0 ;;
esac
case "$AUTOSTART_GNOBLIN" in
    0|false|FALSE|no|NO) AUTOSTART_GNOBLIN=0 ;;
    *) AUTOSTART_GNOBLIN=1 ;;
esac

if [ -f "$MUTTER_BUILD_DIR/src/libmutter-17.so.0" ]; then
    MUTTER_LD_PATH="$MUTTER_BUILD_DIR/src:$MUTTER_BUILD_DIR/clutter/clutter:$MUTTER_BUILD_DIR/cogl/cogl:$MUTTER_BUILD_DIR/mtk/mtk"
    export LD_LIBRARY_PATH="$MUTTER_LD_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    MUTTER_TYPELIB_PATH="$MUTTER_BUILD_DIR/src:$MUTTER_BUILD_DIR/clutter/clutter:$MUTTER_BUILD_DIR/cogl/cogl:$MUTTER_BUILD_DIR/mtk/mtk"
    if [ -f "$MUTTER_BUILD_DIR/src/Meta-17.typelib" ]; then
        export GI_TYPELIB_PATH="$MUTTER_TYPELIB_PATH${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
    fi
    echo ">> using patched mutter build: $MUTTER_BUILD_DIR"

    LD_TRACE_LOADED_OBJECTS=1 "$GNOME_SHELL_BIN" >"$LOADER_LOG" 2>&1 || true
    if grep -q "$MUTTER_BUILD_DIR/src/libmutter-17.so" "$LOADER_LOG"; then
        ok "gnome-shell resolves libmutter-17 from the patched build"
    else
        fail "gnome-shell did not resolve libmutter-17 from $MUTTER_BUILD_DIR (see $LOADER_LOG)"
    fi
else
    echo ">> MUTTER_BUILD_DIR has no libmutter build; testing installed gnome-shell/mutter"
fi

NESTED_DISPLAY="${NESTED_DISPLAY:-gnoblin-test-$$}"
DUMMY_MODE_SPECS="${MUTTER_DEBUG_DUMMY_MODE_SPECS:-1200x800}"
IFS=',' read -r -a DUMMY_MODES <<< "$DUMMY_MODE_SPECS"
SHELL_MODE_ARGS=(--headless)

if [ "$VISIBLE" -eq 1 ]; then
    if "$GNOME_SHELL_BIN" --help 2>&1 | grep -q -- '--devkit'; then
        SHELL_MODE_ARGS=(--devkit)
    else
        echo "test-nested: visible mode requires gnome-shell --devkit support" >&2
        exit 1
    fi
fi
if [ "$VISIBLE" -eq 0 ] ||
   [ "${GNOBLIN_DEVKIT_EXTRA_VIRTUAL_MONITORS:-0}" = "1" ]; then
    for mode in "${DUMMY_MODES[@]}"; do
        mode="${mode#"${mode%%[![:space:]]*}"}"
        mode="${mode%"${mode##*[![:space:]]}"}"
        [ -n "$mode" ] && SHELL_MODE_ARGS+=(--virtual-monitor "$mode")
    done
fi

echo ">> launching isolated gnome-shell on $NESTED_DISPLAY"
if [ "$VISIBLE" -eq 1 ]; then
    echo ">> visible mode enabled via Mutter Devkit"
fi
SHELL_ENV=(NO_AT_BRIDGE=1)
if [ "$VISIBLE" -eq 1 ] && [ -n "${GNOBLIN_DEVKIT_SHOW_HOST_CURSOR:-}" ]; then
    SHELL_ENV+=(GNOBLIN_DEVKIT_SHOW_HOST_CURSOR="$GNOBLIN_DEVKIT_SHOW_HOST_CURSOR")
fi
if [ "$AUTOSTART_GNOBLIN" -eq 1 ]; then
    echo "   skip  legacy GNOME Shell gnoblin autostart is retired; running installed clients manually"
    AUTOSTART_GNOBLIN=0
fi
SHELL_ENV+=(GNOBLIN_LAYER_SHELL_AUTOSTART=none)

env "${SHELL_ENV[@]}" dbus-run-session -- \
    "$GNOME_SHELL_BIN" \
        --wayland \
        "${SHELL_ARGS[@]}" \
        "${SHELL_MODE_ARGS[@]}" \
        --wayland-display "$NESTED_DISPLAY" >"$SHELL_LOG" 2>&1 &
SHELL_PID=$!
trap 'kill "$SHELL_PID" 2>/dev/null; wait "$SHELL_PID" 2>/dev/null' EXIT

shell_is_running() {
    local stat

    kill -0 "$SHELL_PID" 2>/dev/null || return 1
    stat="$(ps -p "$SHELL_PID" -o stat= 2>/dev/null || true)"
    [ -n "$stat" ] || return 1
    case "$stat" in
        *Z*) return 1 ;;
    esac
    return 0
}

abort_if_shell_stopped() {
    if shell_is_running; then
        return
    fi

    echo "!! nested shell exited early; see $SHELL_LOG" >&2
    tail -n 80 "$SHELL_LOG" >&2
    exit 1
}

NEW_DISP=""
for _ in $(seq 1 60); do
    sleep 0.5
    abort_if_shell_stopped

    if [ -S "$XDG_RUNTIME_DIR/$NESTED_DISPLAY" ]; then
        NEW_DISP="$NESTED_DISPLAY"
    fi
    [ -n "$NEW_DISP" ] && break
done

if [ -z "$NEW_DISP" ]; then
    fail "could not find nested compositor socket"
else
    ok "nested compositor socket: $NEW_DISP"
fi
sleep 1
abort_if_shell_stopped

run_layer_client() {
    local name="$1"
    shift
    local log="$OUT/$name.log"
    local status

    echo ">> running real layer-shell client: $name"
    abort_if_shell_stopped
    env -u DISPLAY \
        WAYLAND_DISPLAY="$NEW_DISP" \
        GDK_BACKEND=wayland \
        XDG_SESSION_TYPE=wayland \
        PATH="$GNOBLIN_PREFIX/bin:$PATH" \
        GSETTINGS_SCHEMA_DIR="$GNOBLIN_PREFIX/share/glib-2.0/schemas" \
        XDG_DATA_DIRS="$GNOBLIN_PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" \
        WAYLAND_DEBUG=1 \
        timeout "${CLIENT_TIMEOUT}s" "$@" >"$log" 2>&1
    status=$?

    if [ "$status" -ne 0 ] && [ "$status" -ne 124 ]; then
        fail "$name exited with $status (see $log)"
        tail -n 40 "$log" >&2
        return
    fi

    if grep -q "zwlr_layer_shell_v1" "$log" &&
       grep -q "zwlr_layer_surface_v1" "$log"; then
        ok "$name bound and used zwlr_layer_shell_v1"
    else
        fail "$name did not use zwlr_layer_shell_v1 (see $log)"
    fi

    if grep -Eiq "protocol error|wl_display@.*error|invalid_surface_state|invalid_size|invalid_anchor|invalid_keyboard_interactivity|invalid_exclusive_edge" "$log"; then
        fail "$name hit a Wayland protocol error (see $log)"
    fi
}

check_layer_log() {
    local name="$1"
    local log="$2"

    if [ ! -f "$log" ]; then
        fail "$name did not produce a log (expected $log)"
        return
    fi

    if grep -q "zwlr_layer_shell_v1" "$log" &&
       grep -q "zwlr_layer_surface_v1" "$log"; then
        ok "$name bound and used zwlr_layer_shell_v1"
    else
        fail "$name did not use zwlr_layer_shell_v1 (see $log)"
        [ -f "$log" ] && tail -n 40 "$log" >&2
    fi

    if grep -Eiq "protocol error|wl_display@.*error|invalid_surface_state|invalid_size|invalid_anchor|invalid_keyboard_interactivity|invalid_exclusive_edge" "$log"; then
        fail "$name hit a Wayland protocol error (see $log)"
    fi
}

check_shell_log_health() {
    if grep -Eiq "CRITICAL|JS ERROR|segmentation fault" "$SHELL_LOG"; then
        fail "nested shell logged a critical/runtime error (see $SHELL_LOG)"
        grep -Ein "CRITICAL|JS ERROR|segmentation fault" "$SHELL_LOG" >&2 || true
    else
        ok "nested shell log has no critical/runtime errors"
    fi
}

wait_for_autostart_component() {
    local name="$1"
    local log="$OUT/autostart/$name.log"

    echo ">> checking layer-shell autostart client: gnoblin-$name"
    for _ in $(seq 1 "$AUTOSTART_TIMEOUT"); do
        sleep 0.5
        abort_if_shell_stopped

        if [ -f "$log" ] && grep -q "zwlr_layer_surface_v1" "$log"; then
            check_layer_log "gnoblin-$name autostart" "$log"
            return
        fi
    done

    fail "gnoblin-$name did not autostart or produce layer-shell traffic (see $log and $SHELL_LOG)"
    [ -f "$log" ] && tail -n 40 "$log" >&2
}

wait_for_shell_startup() {
    echo ">> waiting for gnome-shell startup-complete"
    for _ in $(seq 1 "$AUTOSTART_TIMEOUT"); do
        sleep 0.5
        abort_if_shell_stopped

        if grep -q "GNOME Shell started at" "$SHELL_LOG"; then
            ok "gnome-shell reached startup-complete"
            return
        fi
    done

    fail "gnome-shell did not reach startup-complete before autostart validation (see $SHELL_LOG)"
}

build_layer_probe() {
    local builddir="$OUT/probe-build"
    local xdg_shell_xml
    local cflags
    local libs

    if ! command -v wayland-scanner >/dev/null 2>&1; then
        fail "wayland-scanner is not installed; cannot build layer-shell probe"
        return 1
    fi
    if ! command -v cc >/dev/null 2>&1; then
        fail "cc is not installed; cannot build layer-shell probe"
        return 1
    fi
    if ! pkg-config --exists wayland-client wayland-protocols; then
        fail "wayland-client/wayland-protocols pkg-config metadata missing"
        return 1
    fi

    xdg_shell_xml="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"
    if [ ! -f "$xdg_shell_xml" ]; then
        fail "xdg-shell protocol XML not found at $xdg_shell_xml"
        return 1
    fi

    mkdir -p "$builddir"
    wayland-scanner client-header \
        "$REPO_ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml" \
        "$builddir/wlr-layer-shell-unstable-v1-client-protocol.h" || return 1
    wayland-scanner private-code \
        "$REPO_ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml" \
        "$builddir/wlr-layer-shell-unstable-v1-protocol.c" || return 1
    wayland-scanner client-header \
        "$xdg_shell_xml" \
        "$builddir/xdg-shell-client-protocol.h" || return 1
    wayland-scanner private-code \
        "$xdg_shell_xml" \
        "$builddir/xdg-shell-protocol.c" || return 1

    cflags="$(pkg-config --cflags wayland-client)"
    libs="$(pkg-config --libs wayland-client)"
    cc -Wall -Wextra -Werror -I "$builddir" \
        $cflags \
        "$REPO_ROOT/scripts/layer-shell-probe.c" \
        "$builddir/wlr-layer-shell-unstable-v1-protocol.c" \
        "$builddir/xdg-shell-protocol.c" \
        -o "$OUT/layer-shell-probe" \
        $libs || {
            fail "failed to build layer-shell probe"
            return 1
        }

    ok "built layer-shell protocol probe"
}

run_probe_case() {
    local mode="$1"
    local log="$OUT/probe-$mode.log"
    local status
    local extra_env=()

    if [ "$mode" = "hold-overlay" ]; then
        extra_env+=(PROBE_HOLD_SECONDS="${PROBE_HOLD_SECONDS:-1}")
    fi

    echo ">> running layer-shell protocol probe: $mode"
    abort_if_shell_stopped
    env -u DISPLAY \
        WAYLAND_DISPLAY="$NEW_DISP" \
        XDG_SESSION_TYPE=wayland \
        "${extra_env[@]}" \
        timeout "${CLIENT_TIMEOUT}s" "$OUT/layer-shell-probe" "$mode" >"$log" 2>&1
    status=$?

    if [ "$status" -eq 0 ]; then
        ok "probe $mode"
    else
        fail "probe $mode exited with $status (see $log)"
        tail -n 40 "$log" >&2
    fi
}

if [ -n "$NEW_DISP" ] && [ "$RUN_CLIENTS" -eq 1 ]; then
    if command -v wayland-info >/dev/null 2>&1; then
        WAYLAND_DISPLAY="$NEW_DISP" wayland-info >"$OUT/wayland-info.log" 2>&1
        grep -q "zwlr_layer_shell_v1" "$OUT/wayland-info.log" \
            && ok "zwlr_layer_shell_v1 advertised" \
            || fail "zwlr_layer_shell_v1 not advertised (see $OUT/wayland-info.log)"
    fi

    if [ "$AUTOSTART_GNOBLIN" -eq 1 ]; then
        wait_for_shell_startup
        wait_for_autostart_component topbar
        wait_for_autostart_component dock
    fi

    if [ "$RUN_PROBE" -eq 1 ]; then
        if build_layer_probe; then
            for mode in \
                map \
                popup \
                invalid-size \
                invalid-anchor \
                invalid-keyboard \
                invalid-exclusive-edge \
                buffer-before-configure \
                ack-before-configure \
                bad-serial-high \
                unknown-serial-low \
                invalid-layer \
                hold-overlay
            do
                run_probe_case "$mode"
            done
        fi
    else
        echo "   skip  layer-shell protocol probe disabled by RUN_PROBE=0"
    fi

    if command -v waybar >/dev/null 2>&1; then
        cat >"$OUT/waybar.json" <<'EOF'
{
  "layer": "top",
  "position": "top",
  "height": 28,
  "modules-left": ["custom/gnoblin"],
  "custom/gnoblin": {
    "format": "gnoblin layer-shell e2e",
    "exec": "printf ok",
    "interval": "once"
  }
}
EOF
        cat >"$OUT/waybar.css" <<'EOF'
* {
  font-family: sans-serif;
  font-size: 13px;
}
window#waybar {
  background: #1d1f21;
  color: #ffffff;
}
EOF
        run_layer_client waybar waybar -c "$OUT/waybar.json" -s "$OUT/waybar.css"
    else
        fail "waybar is not installed; cannot run real bar e2e"
    fi

    if command -v wofi >/dev/null 2>&1; then
        run_layer_client wofi wofi --show drun --prompt "gnoblin e2e"
    else
        echo "   skip  wofi not installed"
    fi

    if [ "$AUTOSTART_GNOBLIN" -eq 0 ] && [ "$RUN_GNOBLIN_CLIENTS" -eq 1 ]; then
        if [ -x "$TOPBAR_BIN" ]; then
            run_layer_client gnoblin-topbar "$TOPBAR_BIN"
        else
            fail "$TOPBAR_BIN is not executable (run just dev-userspace)"
        fi

        if [ -x "$DOCK_BIN" ]; then
            run_layer_client gnoblin-dock "$DOCK_BIN"
        else
            fail "$DOCK_BIN is not executable (run just dev-userspace)"
        fi
    elif [ "$AUTOSTART_GNOBLIN" -eq 0 ]; then
        echo "   skip  gnoblin clients in legacy GNOME Shell test (set RUN_GNOBLIN_CLIENTS=1 to opt in)"
    fi
elif [ "$RUN_CLIENTS" -eq 0 ]; then
    echo "   skip  layer-shell clients disabled by RUN_CLIENTS=0"
fi

echo ">> capturing screenshot"
if [ -n "$NEW_DISP" ] && command -v grim >/dev/null 2>&1; then
    if env -u DISPLAY WAYLAND_DISPLAY="$NEW_DISP" grim "$OUT/nested.png" 2>/dev/null; then
        ok "saved $OUT/nested.png"
    else
        echo "   skip  nested screenshot capture unavailable"
    fi
elif command -v gnome-screenshot >/dev/null 2>&1; then
    gnome-screenshot -f "$OUT/nested.png" 2>/dev/null && ok "saved $OUT/nested.png" || true
fi

if [ "$HOLD_OPEN_SECONDS" -gt 0 ]; then
    echo ">> holding nested shell open for ${HOLD_OPEN_SECONDS}s"
    sleep "$HOLD_OPEN_SECONDS"
    abort_if_shell_stopped
fi

echo ">> shutting nested shell down"
kill "$SHELL_PID" 2>/dev/null
wait "$SHELL_PID" 2>/dev/null
trap - EXIT

check_shell_log_health

[ "$rc" -eq 0 ] && echo "TIER 3: PASS" || echo "TIER 3: FAIL (see $OUT)"
exit "$rc"
