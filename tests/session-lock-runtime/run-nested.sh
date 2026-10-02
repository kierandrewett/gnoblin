#!/usr/bin/env bash
# Exercise ext-session-lock-v1 through a supervised, headless Gnoblin session.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_SESSION_LOCK_PREFIX:-$ROOT/install}"
RUNTIME="${GNOBLIN_RUNTIME_BIN:-$ROOT/build/ninja/gnoblin}"
if [[ ! -x "$RUNTIME" ]]; then
    RUNTIME="$PREFIX/bin/gnoblin"
fi

for command in cc dbus-run-session pkg-config wayland-scanner; do
    command -v "$command" >/dev/null || {
        echo "SKIP: $command is required for the session-lock runtime check" >&2
        exit 77
    }
done
pkg-config --exists wayland-client || {
    echo "SKIP: wayland-client development files are required" >&2
    exit 77
}
[[ -x "$RUNTIME" ]] || {
    echo "SKIP: no Gnoblin runtime found at $RUNTIME" >&2
    exit 77
}

BUILD_ROOT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
HOST_RUNTIME_DIR="$BUILD_ROOT"
mkdir -p "$BUILD_ROOT"
BUILD="$(mktemp -d "$BUILD_ROOT/gnoblin-lock.XXXXXX")"
cleanup() { rm -rf -- "$BUILD"; }
trap cleanup EXIT INT TERM HUP

wayland-scanner client-header "$ROOT/src/protocols/session-lock/ext-session-lock-v1.xml" \
    "$BUILD/ext-session-lock-v1-client-protocol.h"
wayland-scanner private-code "$ROOT/src/protocols/session-lock/ext-session-lock-v1.xml" \
    "$BUILD/ext-session-lock-v1-protocol.c"
read -r -a WAYLAND_FLAGS <<<"$(pkg-config --cflags --libs wayland-client)"
cc -Wall -Wextra -Werror -I"$BUILD" \
    "$ROOT/tests/session-lock-runtime/session-lock-smoke.c" \
    "$BUILD/ext-session-lock-v1-protocol.c" \
    "${WAYLAND_FLAGS[@]}" -o "$BUILD/session-lock-smoke"

mkdir -p "$BUILD/config/gnoblin" "$BUILD/runtime" "$BUILD/home" "$BUILD/cache" "$BUILD/state"
chmod 700 "$BUILD/runtime"
cat >"$BUILD/config/gnoblin/init.lua" <<'LUA'
return { protocols = { ["ext-session-lock"] = true } }
LUA
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GNOBLIN_PREFIX="$PREFIX"
export GNOBLIN_CONFIG="$BUILD/config/gnoblin/init.lua"
export XDG_RUNTIME_DIR="$BUILD/runtime" HOME="$BUILD/home"
export XDG_CONFIG_HOME="$BUILD/config" XDG_CACHE_HOME="$BUILD/cache"
export XDG_STATE_HOME="$BUILD/state"
export WAYLAND_DISPLAY="gl-$$"
export GIO_USE_VFS=local GSETTINGS_BACKEND=memory GTK_A11Y=none
if [[ -S "$HOST_RUNTIME_DIR/pipewire-0" ]]; then
    ln -s "$HOST_RUNTIME_DIR/pipewire-0" "$XDG_RUNTIME_DIR/pipewire-0"
fi

DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$BUILD" "$ROOT")"
export ROOT BUILD PREFIX RUNTIME DBUS_CONF
set +e
dbus-run-session --config-file="$DBUS_CONF" -- bash -euo pipefail -c '
  devkit_capture_refused_during_lock() {
    grep -q "Devkit viewer exited; keeping the compositor session alive" "$BUILD/runtime.log" ||
      grep -q "Screen capture is unavailable while the session is locked" "$BUILD/runtime.log" "$BUILD/dbus.log"
  }
  cleanup_inner() {
    if [[ -n "${SUPERVISOR_PID:-}" ]]; then
      kill "$SUPERVISOR_PID" 2>/dev/null || true
      wait "$SUPERVISOR_PID" 2>/dev/null || true
    fi
    if [[ -n "${OWNER_PID:-}" ]]; then
      kill -KILL "$OWNER_PID" 2>/dev/null || true
      wait "$OWNER_PID" 2>/dev/null || true
    fi
  }
  trap cleanup_inner EXIT INT TERM HUP

  GNOBLIN_DEVKIT_KEEP_SESSION=1 "$RUNTIME" --devkit --wayland-display "$WAYLAND_DISPLAY" --no-xwayland >"$BUILD/runtime.log" 2>&1 &
  SUPERVISOR_PID=$!
  for _ in $(seq 1 200); do
    if [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && "$PREFIX/bin/gnoblinctl" --timeout 1 ping >/dev/null 2>&1; then
      break
    fi
    if ! kill -0 "$SUPERVISOR_PID" 2>/dev/null; then
      cat "$BUILD/runtime.log" >&2
      exit 1
    fi
    sleep 0.1
  done
  [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || { cat "$BUILD/runtime.log" >&2; exit 1; }
  "$BUILD/session-lock-smoke" probe
  if ! "$BUILD/session-lock-smoke" lock-unlock; then
    if devkit_capture_refused_during_lock; then
      echo "SKIP: the nested devkit cannot confirm lock presentation after screen capture is refused"
      exit 77
    fi
    exit 1
  fi
  if devkit_capture_refused_during_lock; then
    echo "SKIP: the nested devkit cannot present additional lock states after its screen-capture stream is refused"
    exit 77
  fi
  "$BUILD/session-lock-smoke" destroy-before-locked
  "$BUILD/session-lock-smoke" lock-hold >"$BUILD/owner.log" 2>&1 &
  OWNER_PID=$!
  for _ in $(seq 1 100); do
    grep -q "^LOCKED:" "$BUILD/owner.log" && break
    sleep 0.1
  done
  grep -q "^LOCKED:" "$BUILD/owner.log" || { cat "$BUILD/owner.log" >&2; exit 1; }
  "$BUILD/session-lock-smoke" second
  kill -KILL "$OWNER_PID"
  wait "$OWNER_PID" 2>/dev/null || true
  OWNER_PID=""
  "$BUILD/session-lock-smoke" takeover
  echo "PASS: ext-session-lock-v1 presentation, contention, death, and takeover"
' >"$BUILD/session.log" 2>"$BUILD/dbus.log"
status=$?
set -e
output="$(cat "$BUILD/session.log")"
if ((status != 0)); then
    printf '%s\n' "$output" >&2
    if ((status == 77)); then
        exit 77
    fi
    tail -n 30 "$BUILD/dbus.log" >&2
    [[ ! -f "$BUILD/runtime.log" ]] || tail -n 100 "$BUILD/runtime.log" >&2
    exit "$status"
fi
grep -E '^(PROBE|CANCELLED|LOCKED|UNLOCKED|SECOND|TAKEOVER|PASS):' <<<"$output"
grep -q 'PROBE: ext_session_lock_manager_v1 available' <<<"$output"
grep -q 'PASS: ext-session-lock-v1 presentation, contention, death, and takeover' <<<"$output"
