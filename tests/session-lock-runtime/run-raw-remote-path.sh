#!/usr/bin/env bash
# Exercise Mutter's private ScreenCast/RemoteDesktop path while an
# ext-session-lock-v1 client is locked. This is compositor-path evidence only:
# it deliberately does not start a portal frontend, PipeWire consumer, or
# RustDesk peer. See README.md for the end-to-end evidence it cannot provide.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MUTTER_BIN="${GNOBLIN_MUTTER_BIN:-}"
MUTTER_PLUGIN="${GNOBLIN_MUTTER_PLUGIN:-}"
MUTTER_LIBDIR="${GNOBLIN_MUTTER_LIBDIR:-}"
SCHEMA_DIR="${GNOBLIN_SCHEMA_DIR:-}"
PIPEWIRE_BIN="${GNOBLIN_PIPEWIRE_BIN:-pipewire}"
PIPEWIRE_CONFIG_DIR="${GNOBLIN_PIPEWIRE_CONFIG_DIR:-}"
SPA_PLUGIN_DIR="${GNOBLIN_SPA_PLUGIN_DIR:-}"

for required in "$MUTTER_BIN" "$MUTTER_PLUGIN" "$SCHEMA_DIR"; do
    if [[ -z "$required" || ! -e "$required" ]]; then
        echo "SKIP: set GNOBLIN_MUTTER_BIN, GNOBLIN_MUTTER_PLUGIN, GNOBLIN_MUTTER_LIBDIR, and GNOBLIN_SCHEMA_DIR for an isolated global-enabled Mutter build"
        exit 77
    fi
done
if [[ -z "$MUTTER_LIBDIR" ]]; then
    echo "SKIP: set GNOBLIN_MUTTER_LIBDIR for an isolated global-enabled Mutter build"
    exit 77
fi
for command in wayland-scanner cc gdbus dbus-run-session python3 "$PIPEWIRE_BIN"; do
    command -v "$command" >/dev/null || { echo "SKIP: missing $command"; exit 77; }
done

BUILD="$(mktemp -d /tmp/gnoblin-lock-remote-path.XXXXXX)"
cleanup() {
    [[ -n "${LOCK_PID:-}" ]] && kill -KILL "$LOCK_PID" 2>/dev/null || true
    [[ -n "${MUTTER_PID:-}" ]] && kill "$MUTTER_PID" 2>/dev/null || true
    rm -rf "$BUILD"
}
trap cleanup EXIT INT TERM HUP

wayland-scanner client-header "$ROOT/src/protocols/session-lock/ext-session-lock-v1.xml" \
    "$BUILD/ext-session-lock-v1-client-protocol.h"
wayland-scanner private-code "$ROOT/src/protocols/session-lock/ext-session-lock-v1.xml" \
    "$BUILD/ext-session-lock-v1-protocol.c"
cc -Wall -Wextra -Werror -I"$BUILD" "$ROOT/tests/session-lock-runtime/session-lock-smoke.c" \
    "$BUILD/ext-session-lock-v1-protocol.c" $(pkg-config --cflags --libs wayland-client) \
    -o "$BUILD/session-lock-smoke"

export ROOT BUILD MUTTER_BIN MUTTER_PLUGIN MUTTER_LIBDIR SCHEMA_DIR
export PIPEWIRE_BIN PIPEWIRE_CONFIG_DIR SPA_PLUGIN_DIR
dbus-run-session -- bash -euo pipefail <<'INNER'
    cleanup_inner() {
        [[ -n "${LOCK_PID:-}" ]] && kill -KILL "$LOCK_PID" 2>/dev/null || true
        [[ -n "${MUTTER_PID:-}" ]] && kill "$MUTTER_PID" 2>/dev/null || true
        [[ -n "${PIPEWIRE_PID:-}" ]] && kill "$PIPEWIRE_PID" 2>/dev/null || true
    }
    trap cleanup_inner EXIT INT TERM HUP

    runtime="$BUILD/runtime"
    mkdir -p "$runtime"
    chmod 700 "$runtime"
    export XDG_RUNTIME_DIR="$runtime"
    export WAYLAND_DISPLAY=gnoblin-lock-remote-path
    export GSETTINGS_BACKEND=memory
    export GSETTINGS_SCHEMA_DIR="$SCHEMA_DIR"
    export GNOME_SHELL_SESSION_MODE=gnoblin
    export LD_LIBRARY_PATH="$MUTTER_LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    [[ -n "$PIPEWIRE_CONFIG_DIR" ]] && export PIPEWIRE_CONFIG_DIR
    [[ -n "$SPA_PLUGIN_DIR" ]] && export SPA_PLUGIN_DIR

    # Keep the PipeWire server and socket inside this disposable runtime
    # directory. The raw Mutter D-Bus APIs need a context to start a remote
    # session; this does not start a portal or use the desktop's PipeWire.
    "$PIPEWIRE_BIN" >"$BUILD/pipewire.log" 2>&1 &
    PIPEWIRE_PID=$!
    for _ in $(seq 1 100); do
        [[ -S "$XDG_RUNTIME_DIR/pipewire-0" ]] && break
        sleep 0.1
    done
    test -S "$XDG_RUNTIME_DIR/pipewire-0"

    "$MUTTER_BIN" --headless --wayland --no-x11 \
        --wayland-display="$WAYLAND_DISPLAY" --virtual-monitor=1280x720@60 \
        --mutter-plugin="$MUTTER_PLUGIN" >"$BUILD/mutter.log" 2>&1 &
    MUTTER_PID=$!
    export MUTTER_PID

    for _ in $(seq 1 100); do
        gdbus introspect --session --dest org.gnome.Mutter.RemoteDesktop \
            --object-path /org/gnome/Mutter/RemoteDesktop >/dev/null 2>&1 && break
        sleep 0.1
    done
    gdbus introspect --session --dest org.gnome.Mutter.RemoteDesktop \
        --object-path /org/gnome/Mutter/RemoteDesktop >/dev/null

    "$BUILD/session-lock-smoke" lock-hold >"$BUILD/locker.log" 2>&1 &
    LOCK_PID=$!
    export LOCK_PID
    for _ in $(seq 1 100); do
        grep -q "^LOCKED:" "$BUILD/locker.log" && break
        sleep 0.1
    done
    grep -q "^LOCKED:" "$BUILD/locker.log"
    # Let the presentation callback settle after the protocol event.
    sleep 1

    python3 "$ROOT/tests/session-lock-runtime/raw-remote-path.py"
INNER
