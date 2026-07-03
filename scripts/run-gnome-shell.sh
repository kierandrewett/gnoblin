#!/usr/bin/env bash
# Boot the *patched gnome-shell* (Gnoblin) headless in the `gnoblin` session
# mode against the installed patched mutter, and verify:
#   (a) it reaches "GNOME Shell started",
#   (b) the compositor advertises zwlr_layer_shell_v1 (chrome can draw),
# then exit cleanly. This is the gnome-shell equivalent of run-devkit.sh's
# `verify` path (which drove the retired C++ gnoblin-shell).
#
# Env: GNOBLIN_PREFIX (default ./install), MONITOR (default 1280x800),
#      SETTLE (seconds to wait for "started", default 25),
#      KEEP=1 to keep the shell alive (prints WAYLAND_DISPLAY, Ctrl-C to exit).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
MONITOR="${MONITOR:-1280x800}"
SETTLE="${SETTLE:-25}"
LAST_LOG=/tmp/gnoblin-gnome-shell-last.log

[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build/install first" >&2; exit 1; }
[ -f "$PREFIX/lib64/libmutter-17.so.0" ] || { echo "no mutter in $PREFIX" >&2; exit 1; }

# --- runtime lookup paths (mirror run-devkit.sh) ---------------------------
export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GDK_BACKEND=wayland
# gnome-shell reads the session mode from this env (main.c:632). We ALSO pass
# --mode=gnoblin below (parsed after, wins) — belt and braces.
export GNOME_SHELL_SESSION_MODE=gnoblin
export XDG_CURRENT_DESKTOP=GNOME:Gnoblin

# --- isolated throwaway state ----------------------------------------------
DK="$(mktemp -d /tmp/gnoblin-gs.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home"
export XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
export GSETTINGS_BACKEND=memory
export GTK_A11Y=none NO_AT_BRIDGE=1

DISP="gnoblin-gs-$$"
SHELL_PID=
cleanup() {
  [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
  [ -n "$SHELL_PID" ] && wait "$SHELL_PID" 2>/dev/null || true
  # sweep any children that joined the nested display
  for proc in /proc/[0-9]*; do
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$env" in *"WAYLAND_DISPLAY=$DISP"*) kill "-KILL" "${proc##*/}" 2>/dev/null || true ;; esac
  done
  [ -f "$DK/shell.log" ] && cp "$DK/shell.log" "$LAST_LOG" 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

# Reuse the devkit's isolated dbus config generator (no host portal leakage).
DBUS_SESSION_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

echo ">> booting patched gnome-shell (mode=gnoblin) headless from $PREFIX ..."
dbus-run-session --config-file="$DBUS_SESSION_CONF" -- \
  "$SHELL_BIN" --headless --wayland --no-x11 --mode=gnoblin \
  --virtual-monitor "$MONITOR" --wayland-display "$DISP" \
  >"$DK/shell.log" 2>&1 &
SHELL_PID=$!

# Wait for the wayland socket to appear.
for _ in $(seq 1 60); do
  sleep 0.5
  [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break
  kill -0 "$SHELL_PID" 2>/dev/null || break
done
if [ ! -S "$XDG_RUNTIME_DIR/$DISP" ]; then
  echo "!! gnome-shell did not create the wayland socket:" >&2
  tail -n 40 "$DK/shell.log" >&2
  cp "$DK/shell.log" "$LAST_LOG" 2>/dev/null || true
  exit 1
fi

# Wait for "started" (the shell's own readiness signal) or a crash.
started=0
for _ in $(seq 1 $((SETTLE * 2))); do
  if grep -qE "GNOME Shell started|running-state.*RUNNING|Shell.* started" "$DK/shell.log" 2>/dev/null; then
    started=1; break
  fi
  kill -0 "$SHELL_PID" 2>/dev/null || { echo "!! gnome-shell exited during startup" >&2; break; }
  sleep 0.5
done

echo "== session mode / startup log lines =="
grep -iE "session.?mode|gnoblin|GNOME Shell started|JS ERROR|JS WARNING|Traceback|error|assert|abort" \
  "$DK/shell.log" 2>/dev/null | head -40 | sed 's/^/   /'

# --- probe advertised globals ----------------------------------------------
probe=/tmp/gnoblin-wl-globals
if [ ! -x "$probe" ] || [ "$ROOT/scripts/wl-globals.c" -nt "$probe" ]; then
  cc "$ROOT/scripts/wl-globals.c" $(pkg-config --cflags --libs wayland-client) -o "$probe" || exit 1
fi
echo "== wlr_/ext_ protocols advertised by gnome-shell =="
WAYLAND_DISPLAY="$DISP" "$probe" | grep -iE "wlr_|ext_" | sort | sed 's/^/   /'
total="$(WAYLAND_DISPLAY="$DISP" "$probe" | wc -l)"
echo "   (+ $total globals total)"
if WAYLAND_DISPLAY="$DISP" "$probe" | grep -q "zwlr_layer_shell_v1"; then
  echo "== LAYER-SHELL: zwlr_layer_shell_v1 IS advertised =="
  layer_ok=1
else
  echo "== LAYER-SHELL: zwlr_layer_shell_v1 NOT found =="
  layer_ok=0
fi

cp "$DK/shell.log" "$LAST_LOG" 2>/dev/null || true

if [ "${KEEP:-0}" = 1 ]; then
  echo ">> KEEP=1: shell alive on WAYLAND_DISPLAY=$DISP (Ctrl-C to exit). log -> $LAST_LOG"
  wait "$SHELL_PID"
fi

if [ "$started" = 1 ] && [ "${layer_ok:-0}" = 1 ]; then
  echo ">> RESULT: PASS (started + layer-shell advertised). log -> $LAST_LOG"
  exit 0
fi
echo ">> RESULT: incomplete (started=$started layer_shell=${layer_ok:-0}). log -> $LAST_LOG" >&2
exit 1
