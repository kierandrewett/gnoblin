#!/usr/bin/env bash
# Prove init.lua gates both a Gnoblin-owned global and the upstream background
# effect global when Gnoblin mode explicitly disables it.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
[ -x "$SHELL_BIN" ] || {
    echo "no gnome-shell in $PREFIX — build first" >&2
    exit 1
}

source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland

DK="$(mktemp -d /tmp/gnoblin-pg.XXXXXX)"
mkdir -p "$DK"/{home,config,cache}
export HOME="$DK/home" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GSETTINGS_BACKEND=memory GTK_A11Y=none
export DISP="gnoblin-pg-$$" GS="$SHELL_BIN"

# init.lua disabling layer shell and the background-effect global.
CONF_FILE="$DK/config/gnoblin/init.lua"
mkdir -p "$(dirname "$CONF_FILE")"
printf 'return {protocols = {["wlr-layer-shell"] = false, ["ext-background-effect-v1"] = false}}\n' >"$CONF_FILE"
export GNOBLIN_CONFIG="$CONF_FILE"

probe="$DK/wl-globals"
compiler_flags="$(pkg-config --cflags --libs wayland-client)" || exit 1
read -r -a compiler_args <<<"$compiler_flags"
cc "$ROOT/tests/wl-globals.c" "${compiler_args[@]}" -o "$probe" || exit 1

cleanup() {
    for proc in /proc/[0-9]*; do
        e="$({ tr '\0' '\n' <"$proc/environ"; } 2>/dev/null || true)"
        case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
    done
    rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1
export DK probe

dbus-run-session --config-file="$DBUS_CONF" -- bash -uo pipefail -c '
  source "$ROOT/tests/gnoblin-test-lib.sh"
  "$GS" --headless --wayland --no-x11 --mode=gnoblin --virtual-monitor 1280x800 \
    --wayland-display "$DISP" >"$DK/shell.log" 2>&1 &
  SHELL_PID=$!
  if ! gnoblin_wait_for_log "$DK/shell.log" "GNOME Shell started" 30; then
    echo "FAIL: shell never reached ready state"
    tail -15 "$DK/shell.log"
    exit 1
  fi
  if ! globals="$(WAYLAND_DISPLAY="$DISP" "$probe")"; then
    echo "FAIL: protocol probe could not complete"
    tail -15 "$DK/shell.log"
    exit 1
  fi
  if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    echo "FAIL: shell exited before protocol assertion"
    tail -15 "$DK/shell.log"
    exit 1
  fi
  if printf "%s\n" "$globals" | grep -q zwlr_layer_shell_v1; then
    echo "FAIL: zwlr_layer_shell_v1 still advertised with wlr-layer-shell=false"
    exit 1
  fi
  if printf "%s\n" "$globals" | grep -q ext_background_effect_manager_v1; then
    echo "FAIL: ext_background_effect_manager_v1 still advertised with ext_background_effect_v1=false"
    exit 1
  fi
  echo "  ok: layer-shell and background-effect globals are hidden by config"
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (init.lua protocol gating)" || echo ">> RESULT: FAIL (rc=$rc)"
exit "$rc"
