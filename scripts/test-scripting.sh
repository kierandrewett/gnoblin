#!/usr/bin/env bash
# Prove the GJS user-scripting layer: drop a script in ~/.config/gnoblin/scripts/,
# confirm it loads and runs, then edit it and ReloadScripts over org.gnoblin.Shell
# and confirm the NEW code ran — no compositor restart.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/gnoblin-state.sh"
LAST_LOG="$(gnoblin_state_dir)/scripting-last.log"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GDK_BACKEND=wayland GNOME_SHELL_SESSION_MODE=gnoblin XDG_CURRENT_DESKTOP=GNOME:Gnoblin

DK="$(mktemp -d /tmp/gnoblin-scr.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GSETTINGS_BACKEND=memory GTK_A11Y=none NO_AT_BRIDGE=1
export DISP="gnoblin-scr-$$" SHELL_LOG="$DK/shell.log"

# Drop a user script (version A) into the config dir the ScriptHost watches.
SCRIPTDIR="$DK/config/gnoblin/scripts"
mkdir -p "$SCRIPTDIR"
printf 'export default (api) => { api.log("SCRIPT version=A"); };\n' > "$SCRIPTDIR/hello.js"
export SCRIPTDIR

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  [ -f "$SHELL_LOG" ] && gnoblin_publish_log "$SHELL_LOG" scripting-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

dbus-run-session --config-file="$CONF" -- bash -uo pipefail -c '
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor 1280x800 --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  gdbus wait --session --timeout 30 org.gnoblin.Shell || { echo "FAIL: shell never up"; tail -20 "$SHELL_LOG"; exit 1; }
  sleep 1

  gnoblin() { gdbus call --session --dest org.gnoblin.Shell --object-path /org/gnoblin/Shell \
               --method "org.gnoblin.Shell.$@" 2>&1; }

  rc=0
  if grep -q "SCRIPT version=A" "$SHELL_LOG"; then echo "  ok: script loaded (version=A)"; else echo "  FAIL: script did not load"; grep -i script "$SHELL_LOG" | tail; rc=1; fi
  echo "ListScripts -> $(gnoblin ListScripts)"
  case "$(gnoblin ListScripts)" in *hello.js*) echo "  ok: ListScripts shows hello.js";; *) echo "  FAIL: not listed"; rc=1;; esac

  printf "export default (api) => { api.log(\"SCRIPT version=B\"); };\n" > "$SCRIPTDIR/hello.js"
  echo "ReloadScripts -> $(gnoblin ReloadScripts)"
  sleep 1
  if grep -q "SCRIPT version=B" "$SHELL_LOG"; then echo "  ok: script HOT-RELOAD picked up new code (version=B)"; else echo "  FAIL: script not reloaded"; grep "SCRIPT version" "$SHELL_LOG"; rc=1; fi

  kill $SHELL_PID 2>/dev/null || true
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (user scripting + hot-reload)" || echo ">> RESULT: FAIL (rc=$rc). log -> $LAST_LOG"
exit "$rc"
