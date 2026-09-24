#!/usr/bin/env bash
# Black-box nested runtime suite for ext-session-lock-v1. See README.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"

if ! command -v wayland-scanner >/dev/null || ! pkg-config --exists wayland-client; then
  echo "SKIP: wayland-scanner and wayland-client development files are required"
  exit 77
fi
if [[ ! -x "$SHELL_BIN" ]]; then
  echo "SKIP: no installed Gnoblin at $SHELL_BIN"
  exit 77
fi

BUILD="$(mktemp -d /tmp/gnoblin-session-lock-runtime.XXXXXX)"
cleanup() {
  [[ -n "${SHELL_PID:-}" ]] && kill "$SHELL_PID" 2>/dev/null || true
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

source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland GIO_USE_VFS=local GSETTINGS_BACKEND=memory GTK_A11Y=none
export WAYLAND_DISPLAY="gnoblin-session-lock-$$"
export XDG_RUNTIME_DIR="$BUILD/runtime"
mkdir -p "$XDG_RUNTIME_DIR" "$BUILD/home" "$BUILD/config" "$BUILD/cache"
chmod 700 "$XDG_RUNTIME_DIR"
export HOME="$BUILD/home" XDG_CONFIG_HOME="$BUILD/config" XDG_CACHE_HOME="$BUILD/cache"
mkdir -p "$XDG_CONFIG_HOME/gnoblin"
printf 'return { protocols = { ["ext-session-lock"] = true } }\n' > "$XDG_CONFIG_HOME/gnoblin/init.lua"
export GNOBLIN_CONFIG="$XDG_CONFIG_HOME/gnoblin/init.lua"
export ROOT BUILD SHELL_BIN

DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$BUILD" "$ROOT")"
dbus-run-session --config-file="$DBUS_CONF" -- bash -euo pipefail -c '
  cleanup_inner() {
    [[ -n "${OWNER_PID:-}" ]] && kill -KILL "$OWNER_PID" 2>/dev/null || true
    [[ -n "${SHELL_PID:-}" ]] && kill "$SHELL_PID" 2>/dev/null || true
  }
  trap cleanup_inner EXIT INT TERM HUP
  "$SHELL_BIN" --headless --wayland --no-x11 --mode=gnoblin --virtual-monitor 1280x800 \
    --wayland-display "$WAYLAND_DISPLAY" >"$BUILD/shell.log" 2>&1 &
  SHELL_PID=$!
  source "$ROOT/tests/gnoblin-test-lib.sh"
  gnoblin_wait_for_log "$BUILD/shell.log" "GNOME Shell started" 30
  "$BUILD/session-lock-smoke" probe
  "$BUILD/session-lock-smoke" destroy-before-locked
  "$BUILD/session-lock-smoke" lock-unlock
  "$BUILD/session-lock-smoke" lock-hold >"$BUILD/owner.log" 2>&1 &
  OWNER_PID=$!
  for _ in $(seq 1 100); do
    grep -q "^LOCKED:" "$BUILD/owner.log" && break
    sleep 0.1
  done
  grep -q "^LOCKED:" "$BUILD/owner.log" || { cat "$BUILD/owner.log"; exit 1; }
  "$BUILD/session-lock-smoke" second
  kill -KILL "$OWNER_PID"
  wait "$OWNER_PID" 2>/dev/null || true
  OWNER_PID=""
  "$BUILD/session-lock-smoke" takeover
  echo "PASS: ext-session-lock-v1 presentation, contention, death, and takeover"
' || {
  status=$?
  [[ $status == 77 ]] && exit 77
  tail -80 "$BUILD/shell.log" >&2 || true
  exit "$status"
}
