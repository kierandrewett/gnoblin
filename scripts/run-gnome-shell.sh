#!/usr/bin/env bash
# Boot the patched gnome-shell headless in a selected session mode and verify
# startup, fatal diagnostics, control-component isolation, and privileged
# protocol exposure. This is the headless smoke test for the patched-GNOME
# stack (see run-gnome-devkit.sh for the visible Gnoblin devkit).
#
# Env: GNOBLIN_PREFIX (default ./install), GNOBLIN_TEST_MODE (default gnoblin),
#      GNOBLIN_TEST_ENV_MODE (default selected mode; may test a stale value),
#      GNOBLIN_EXPECT_PRIVILEGED_PROTOCOLS (default 1 in gnoblin, 0 otherwise),
#      GNOBLIN_TEST_CLIENT (optional Wayland client invoked after startup),
#      GNOBLIN_TEST_DBUS_CLIENT (optional D-Bus client invoked after startup),
#      GNOBLIN_TEST_EXTENSION_ROOT (optional directory of system extension fixtures),
#      GNOBLIN_TEST_GSETTINGS_BACKEND (default memory),
#      GNOBLIN_TEST_DISABLE_NOTIFICATIONS=1 to seed that feature as disabled,
#      MONITOR (default 1280x800), SETTLE (startup timeout seconds, default 25),
#      KEEP=1 to keep the shell alive (prints WAYLAND_DISPLAY, Ctrl-C to exit).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/gnoblin-state.sh"
source "$ROOT/scripts/gnoblin-test-lib.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR
LAST_LOG="$GNOBLIN_STATE_DIR/gnome-shell-last.log"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
MONITOR="${MONITOR:-1280x800}"
SETTLE="${SETTLE:-25}"
MODE="${GNOBLIN_TEST_MODE:-gnoblin}"
ENV_MODE="${GNOBLIN_TEST_ENV_MODE:-$MODE}"
if [ -n "${GNOBLIN_EXPECT_PRIVILEGED_PROTOCOLS:-}" ]; then
  EXPECT_PRIVILEGED_PROTOCOLS="$GNOBLIN_EXPECT_PRIVILEGED_PROTOCOLS"
elif [ "$MODE" = gnoblin ]; then
  EXPECT_PRIVILEGED_PROTOCOLS=1
else
  EXPECT_PRIVILEGED_PROTOCOLS=0
fi

[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build/install first" >&2; exit 1; }
[ -f "$PREFIX/${GNOBLIN_LIBDIR:-lib64}/libmutter-17.so.0" ] || { echo "no mutter in $PREFIX" >&2; exit 1; }

# --- runtime lookup paths (shared with run-gnome-devkit.sh, the installed
# login wrappers) -------------------------------------------------------
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland
# Keep the launcher-provided environment explicit. GNOME Shell must replace a
# stale value with the effective command-line mode before Mutter initialises.
export GNOME_SHELL_SESSION_MODE="$ENV_MODE"
if [ "$MODE" = gnoblin ]; then
  export XDG_CURRENT_DESKTOP=GNOME:Gnoblin
else
  export XDG_CURRENT_DESKTOP=GNOME
fi

# --- isolated throwaway state ----------------------------------------------
DK="$(mktemp -d /tmp/gnoblin-gs.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home"
export XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
export GSETTINGS_BACKEND="${GNOBLIN_TEST_GSETTINGS_BACKEND:-memory}"
export GTK_A11Y=none NO_AT_BRIDGE=1

case "${GNOBLIN_TEST_DISABLE_NOTIFICATIONS:-0}" in
  0) ;;
  1)
    gsettings set org.gnoblin.shell disabled-features "['notifications']" || exit 1
    ;;
  *)
    echo "!! GNOBLIN_TEST_DISABLE_NOTIFICATIONS must be 0 or 1" >&2
    exit 1
    ;;
esac

if [ -n "${GNOBLIN_TEST_EXTENSION_ROOT:-}" ]; then
  if [ ! -d "$GNOBLIN_TEST_EXTENSION_ROOT" ]; then
    echo "!! extension fixture root is not a directory: $GNOBLIN_TEST_EXTENSION_ROOT" >&2
    exit 1
  fi

  extension_data="$DK/test-system/gnome-shell/extensions"
  mkdir -p "$extension_data"
  extension_count=0
  for extension in "$GNOBLIN_TEST_EXTENSION_ROOT"/*; do
    [ -d "$extension" ] || continue
    cp -a -- "$extension" "$extension_data/"
    extension_count=$((extension_count + 1))
  done
  if [ "$extension_count" -eq 0 ]; then
    echo "!! extension fixture root contains no extension directories: $GNOBLIN_TEST_EXTENSION_ROOT" >&2
    exit 1
  fi
  export XDG_DATA_DIRS="$DK/test-system:$XDG_DATA_DIRS"
fi

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
  [ -f "$DK/shell.log" ] && gnoblin_publish_log "$DK/shell.log" gnome-shell-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

# Reuse the devkit's isolated dbus config generator (no host portal leakage).
DBUS_SESSION_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1
BUS_ADDRESS_FILE="$DK/bus-address"

echo ">> booting patched gnome-shell (mode=$MODE) headless from $PREFIX ..."
dbus-run-session --config-file="$DBUS_SESSION_CONF" -- \
  bash -c 'printf "%s\n" "$DBUS_SESSION_BUS_ADDRESS" > "$1"; shift; exec "$@"' \
  gnoblin-shell "$BUS_ADDRESS_FILE" \
  "$SHELL_BIN" --headless --wayland --no-x11 --mode="$MODE" \
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
  gnoblin_publish_log "$DK/shell.log" gnome-shell-last.log 2>/dev/null || true
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
grep -iE "session.?mode|gnoblin|GNOME Shell started|CRITICAL|JS ERROR|JS WARNING|Traceback|error|assert|abort" \
  "$DK/shell.log" 2>/dev/null | head -40 | sed 's/^/   /'
fatal=0
if gnoblin_log_has_fatal "$DK/shell.log" >/dev/null; then
  fatal=1
fi

# --- probe advertised globals ----------------------------------------------
probe="$DK/wl-globals"
cc "$ROOT/scripts/wl-globals.c" $(pkg-config --cflags --libs wayland-client) -o "$probe" || exit 1
globals="$(WAYLAND_DISPLAY="$DISP" "$probe")"
echo "== wlr_/ext_ protocols advertised by gnome-shell =="
printf '%s\n' "$globals" | grep -iE "wlr_|ext_" | sort | sed 's/^/   /'
total="$(printf '%s\n' "$globals" | wc -l)"
echo "   (+ $total globals total)"

privileged_protocols=(
  ext_data_control_manager_v1
  ext_foreign_toplevel_list_v1
  ext_idle_notifier_v1
  zwlr_foreign_toplevel_manager_v1
  zwlr_gamma_control_manager_v1
  zwlr_layer_shell_v1
  zwlr_output_power_manager_v1
  zwlr_screencopy_manager_v1
)
protocol_scope_ok=1
for protocol in "${privileged_protocols[@]}"; do
  if printf '%s\n' "$globals" | grep -q "^$protocol "; then
    present=1
  else
    present=0
  fi
  if [ "$present" != "$EXPECT_PRIVILEGED_PROTOCOLS" ]; then
    protocol_scope_ok=0
    echo "!! $protocol exposure mismatch: expected=$EXPECT_PRIVILEGED_PROTOCOLS actual=$present" >&2
  fi
done

control_present=0
if [ -s "$BUS_ADDRESS_FILE" ]; then
  bus_address="$(cat "$BUS_ADDRESS_FILE")"
  if DBUS_SESSION_BUS_ADDRESS="$bus_address" gdbus call --session \
      --dest org.gnoblin.Shell \
      --object-path /org/gnoblin/Shell \
      --method org.gnoblin.Shell.Ping >/dev/null 2>&1; then
    control_present=1
  fi
fi
if { [ "$MODE" = gnoblin ] && [ "$control_present" = 1 ]; } ||
   { [ "$MODE" != gnoblin ] && [ "$control_present" = 0 ]; }; then
  control_scope_ok=1
else
  control_scope_ok=0
  echo "!! gnoblin control scope mismatch: mode=$MODE present=$control_present" >&2
fi

dbus_client_ok=1
if [ -n "${GNOBLIN_TEST_DBUS_CLIENT:-}" ]; then
  if [ ! -x "$GNOBLIN_TEST_DBUS_CLIENT" ]; then
    echo "!! D-Bus test client is not executable: $GNOBLIN_TEST_DBUS_CLIENT" >&2
    dbus_client_ok=0
  elif [ -z "${bus_address:-}" ]; then
    echo "!! isolated D-Bus address is unavailable" >&2
    dbus_client_ok=0
  elif ! GNOBLIN_ACTIVE_MODE="$MODE" DBUS_SESSION_BUS_ADDRESS="$bus_address" "$GNOBLIN_TEST_DBUS_CLIENT"; then
    dbus_client_ok=0
  fi
fi

client_ok=1
if [ -n "${GNOBLIN_TEST_CLIENT:-}" ]; then
  if [ ! -x "$GNOBLIN_TEST_CLIENT" ]; then
    echo "!! protocol test client is not executable: $GNOBLIN_TEST_CLIENT" >&2
    client_ok=0
  elif ! WAYLAND_DISPLAY="$DISP" "$GNOBLIN_TEST_CLIENT"; then
    client_ok=0
  elif ! timeout 5s env WAYLAND_DISPLAY="$DISP" "$probe" >/dev/null; then
    echo "!! compositor did not respond after protocol client disconnect" >&2
    client_ok=0
  elif ! kill -0 "$SHELL_PID" 2>/dev/null; then
    echo "!! gnome-shell exited after protocol client disconnect" >&2
    client_ok=0
  fi
fi

# A client may expose diagnostics after the initial startup scan.
if gnoblin_log_has_fatal "$DK/shell.log" >/dev/null; then
  fatal=1
fi

gnoblin_publish_log "$DK/shell.log" gnome-shell-last.log 2>/dev/null || true

if [ "${KEEP:-0}" = 1 ]; then
  echo ">> KEEP=1: shell alive on WAYLAND_DISPLAY=$DISP (Ctrl-C to exit). log -> $LAST_LOG"
  wait "$SHELL_PID"
fi

if [ "$started" = 1 ] && [ "$protocol_scope_ok" = 1 ] &&
   [ "$control_scope_ok" = 1 ] && [ "$client_ok" = 1 ] &&
   [ "$dbus_client_ok" = 1 ] && [ "$fatal" = 0 ]; then
  echo ">> RESULT: PASS (mode=$MODE started, protocol/control scope correct, clients=$client_ok/$dbus_client_ok, no fatal diagnostics). log -> $LAST_LOG"
  exit 0
fi
echo ">> RESULT: incomplete (mode=$MODE started=$started protocol_scope=$protocol_scope_ok control_scope=$control_scope_ok clients=$client_ok/$dbus_client_ok fatal_diagnostics=$fatal). log -> $LAST_LOG" >&2
exit 1
