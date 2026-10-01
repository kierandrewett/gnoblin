#!/usr/bin/env bash
# Start a supervised Gnoblin compositor in Mutter's nested development viewer.
# The viewer is a window on the current Wayland desktop; clients launched from
# the terminal target the private Gnoblin display.
set -euo pipefail
ulimit -c 0

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/gnoblin-state.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
RUNTIME="$PREFIX/bin/gnoblin"
GNOBLINCTL="$PREFIX/bin/gnoblinctl"
CONFIG_SOURCE="${GNOBLIN_DEVKIT_CONFIG_SOURCE:-}"
unset GNOBLIN_DEVKIT_CONFIG_SOURCE
if [[ -z ${WAYLAND_DISPLAY:-} ]]; then
    echo 'run-gnoblin-devkit: start it from a Wayland desktop.' >&2
    exit 1
fi

# The viewer executable alone does not prove that Gnoblin's compositor accepts
# devkit options. Check the exact binary that the session will launch.
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
compositor_help=''
if [[ -x "$PREFIX/bin/gnoblin-mutter" ]]; then
    compositor_help="$("$PREFIX/bin/gnoblin-mutter" --help 2>&1 || true)"
fi
prepare_devkit=true
if [[ -x "$PREFIX/libexec/mutter-devkit" ]] &&
    grep -Fq -- '--devkit' <<<"$compositor_help"; then
    prepare_devkit=false
fi
if "$prepare_devkit"; then
    if [[ -d "$PREFIX" && ! -w "$PREFIX" ]]; then
        echo 'This prefix has no nested development viewer. Use a writable source-build prefix for --preview.' >&2
        exit 1
    fi
    GNOBLIN_DEVKIT=enabled "$ROOT/build.sh" --prefix "$PREFIX" --target gnoblin-session
fi
[[ -x "$RUNTIME" && -x "$GNOBLINCTL" ]] || {
    echo "No standalone Gnoblin build found in $PREFIX. Run './build.sh' first." >&2
    exit 1
}

HOST_WAYLAND="$WAYLAND_DISPLAY"
HOST_RUNTIME="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
case "$HOST_WAYLAND" in
    /* | "") ;;
    *) HOST_WAYLAND="$HOST_RUNTIME/$HOST_WAYLAND" ;;
esac
DISP="gnoblin-devkit-$$"
DEVKIT_TMP_ROOT="$HOST_RUNTIME"
mkdir -p "$DEVKIT_TMP_ROOT"
DK="$(mktemp -d "$DEVKIT_TMP_ROOT/gnoblin-devkit.XXXXXX")"
mkdir -m 700 "$DK"/{runtime,home,config,data,cache,state}
RUNTIME_PID=''
DBUS_PID=''
cleaned=''
cleanup() {
    if [[ -n $cleaned ]]; then
        return 0
    fi
    cleaned=1
    if [[ -n $RUNTIME_PID ]]; then
        kill -- "-$RUNTIME_PID" 2>/dev/null || true
        kill "$RUNTIME_PID" 2>/dev/null || true
        wait "$RUNTIME_PID" 2>/dev/null || true
    fi
    [[ -z $DBUS_PID ]] || kill "$DBUS_PID" 2>/dev/null || true
    if [[ -f "$DK/runtime.log" ]]; then
        gnoblin_publish_log "$DK/runtime.log" devkit-last.log 2>/dev/null || true
    fi
    rm -rf -- "$DK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if [[ -n $CONFIG_SOURCE ]]; then
    [[ -d "$CONFIG_SOURCE" ]] || {
        echo "run-gnoblin-devkit: config snapshot does not exist: $CONFIG_SOURCE" >&2
        exit 1
    }
    cp -a -- "$CONFIG_SOURCE"/. "$DK/config/"
fi
export XDG_RUNTIME_DIR="$DK/runtime"
export HOME="$DK/home" XDG_CONFIG_HOME="$DK/config" XDG_DATA_HOME="$DK/data"
export XDG_CACHE_HOME="$DK/cache" XDG_STATE_HOME="$DK/state"
export GNOBLIN_COMPOSITOR_SOCKET="$XDG_RUNTIME_DIR/compositor-v1.sock"
unset GNOBLIN_CONFIG
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GTK_A11Y=none NO_AT_BRIDGE=1
export GDK_BACKEND=wayland QT_QPA_PLATFORM=wayland CLUTTER_BACKEND=wayland
export MOZ_ENABLE_WAYLAND=1

# Keep PipeWire reachable while all other user-session state stays private.
if [[ -S "$HOST_RUNTIME/pipewire-0" ]]; then
    ln -s "$HOST_RUNTIME/pipewire-0" "$XDG_RUNTIME_DIR/pipewire-0"
fi

export WAYLAND_DISPLAY="$HOST_WAYLAND"
export XDG_CURRENT_DESKTOP=Gnoblin XDG_SESSION_DESKTOP=gnoblin

# Use a private bus so activation variables and session services from the host
# GNOME session cannot leak into the nested compositor.
DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1
DBUS_PID_FILE="$DK/dbus.pid"
DBUS_SESSION_BUS_ADDRESS="$(dbus-daemon --config-file="$DBUS_CONF" --print-address --fork --print-pid=3 3>"$DBUS_PID_FILE")" || exit 1
export DBUS_SESSION_BUS_ADDRESS
DBUS_PID="$(cat "$DBUS_PID_FILE" 2>/dev/null || true)"

# The host display stays visible to Mutter for its devkit window. Only commands
# launched from the nested terminal receive the private runtime directory and
# nested Wayland display.
setsid "$RUNTIME" --devkit --wayland-display "$DISP" >"$DK/runtime.log" 2>&1 &
RUNTIME_PID=$!

ready=false
for _ in $(seq 1 300); do
    if ! kill -0 "$RUNTIME_PID" 2>/dev/null; then
        echo 'Gnoblin runtime exited before its control socket was ready:' >&2
        tail -n 30 "$DK/runtime.log" >&2
        exit 1
    fi
    if "$GNOBLINCTL" --timeout 1 ping >/dev/null 2>&1 && [[ -S "$XDG_RUNTIME_DIR/$DISP" ]]; then
        ready=true
        break
    fi
    sleep 0.1
done
if ! "$ready"; then
    echo 'Gnoblin did not open its private display and control socket:' >&2
    tail -n 30 "$DK/runtime.log" >&2
    exit 1
fi
echo "Gnoblin is ready on nested Wayland display $DISP."

WAYLAND_DISPLAY="$DISP" dbus-update-activation-environment \
    WAYLAND_DISPLAY GDK_BACKEND QT_QPA_PLATFORM CLUTTER_BACKEND \
    XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP GTK_A11Y NO_AT_BRIDGE GIO_USE_VFS \
    2>/dev/null || true

if [[ -n ${GNOBLIN_DEVKIT_EXEC:-} ]]; then
    WAYLAND_DISPLAY="$DISP" bash -c "$GNOBLIN_DEVKIT_EXEC"
    exit $?
fi

terminal="${1:-}"
if [[ -z $terminal ]]; then
    for candidate in foot kitty alacritty wezterm gnome-terminal konsole xterm; do
        if command -v "$candidate" >/dev/null 2>&1; then
            terminal="$candidate"
            break
        fi
    done
fi
if [[ -z $terminal ]]; then
    echo 'No terminal found. Install foot, kitty, or alacritty, or pass its name.' >&2
    exit 1
fi

inner="export XDG_RUNTIME_DIR='${XDG_RUNTIME_DIR}'
export WAYLAND_DISPLAY='${DISP}'
cat <<'EOF'
Gnoblin devkit

This terminal is shown on your host desktop. Commands run here target the
nested Gnoblin session. Start a shell integration, for example:

    qs -p ~/src/my-shell

Inspect the session with: gnoblinctl ping | version | reload
Close this terminal to stop the nested session.
EOF
exec bash -i"
case "$terminal" in
    alacritty | wezterm | konsole | xterm) XDG_RUNTIME_DIR="$HOST_RUNTIME" WAYLAND_DISPLAY="$HOST_WAYLAND" "$terminal" -e bash -c "$inner" ;;
    gnome-terminal) XDG_RUNTIME_DIR="$HOST_RUNTIME" WAYLAND_DISPLAY="$HOST_WAYLAND" "$terminal" -- bash -c "$inner" ;;
    *) XDG_RUNTIME_DIR="$HOST_RUNTIME" WAYLAND_DISPLAY="$HOST_WAYLAND" "$terminal" bash -c "$inner" ;;
esac
