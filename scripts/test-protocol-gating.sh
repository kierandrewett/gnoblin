#!/usr/bin/env bash
# Prove gnoblin.conf [protocols] gating: with wlr-layer-shell disabled, the
# compositor must NOT advertise zwlr_layer_shell_v1. (The default/enabled case is
# covered by run-gnome-shell.sh, which asserts it IS advertised.)
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GNOME_SHELL_SESSION_MODE=gnoblin GDK_BACKEND=wayland

DK="$(mktemp -d /tmp/gnoblin-pg.XXXXXX)"
mkdir -p "$DK"/{home,config,cache}
export HOME="$DK/home" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GSETTINGS_BACKEND=memory GTK_A11Y=none
export DISP="gnoblin-pg-$$" GS="$SHELL_BIN"

# gnoblin.conf disabling the layer-shell protocol.
CONF_FILE="$DK/config/gnoblin/gnoblin.conf"
mkdir -p "$(dirname "$CONF_FILE")"
printf '[protocols]\nwlr-layer-shell = false\n' > "$CONF_FILE"
export GNOBLIN_CONFIG="$CONF_FILE"

probe=/tmp/gnoblin-wl-globals
if [ ! -x "$probe" ] || [ "$ROOT/scripts/wl-globals.c" -nt "$probe" ]; then
  cc "$ROOT/scripts/wl-globals.c" $(pkg-config --cflags --libs wayland-client) -o "$probe" || exit 1
fi

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1
export DK probe

dbus-run-session --config-file="$DBUS_CONF" -- bash -uo pipefail -c '
  "$GS" --headless --wayland --no-x11 --mode=gnoblin --virtual-monitor 1280x800 \
    --wayland-display "$DISP" >"$DK/shell.log" 2>&1 &
  for i in $(seq 1 60); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.5; done
  [ -S "$XDG_RUNTIME_DIR/$DISP" ] || { echo "FAIL: shell socket never appeared"; tail -15 "$DK/shell.log"; exit 1; }
  sleep 3
  if WAYLAND_DISPLAY="$DISP" "$probe" | grep -q zwlr_layer_shell_v1; then
    echo "  FAIL: zwlr_layer_shell_v1 STILL advertised with wlr-layer-shell=false"
    exit 1
  fi
  echo "  ok: zwlr_layer_shell_v1 NOT advertised (config gating works)"
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (gnoblin.conf [protocols] gating)" || echo ">> RESULT: FAIL (rc=$rc)"
exit "$rc"
