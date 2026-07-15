#!/usr/bin/env bash
# Prove the GJS user-scripting layer: drop a script in ~/.config/gnoblin/scripts/,
# confirm it loads and runs, then edit it and ReloadScripts over org.gnoblin.Shell
# and confirm the NEW code ran — no compositor restart.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
source "$ROOT/scripts/gnoblin-state.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR
LAST_LOG="$GNOBLIN_STATE_DIR/scripting-last.log"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland

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
  source "$ROOT/scripts/gnoblin-test-lib.sh"
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor 1280x800 --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  gdbus wait --session --timeout 30 org.gnoblin.Shell || { echo "FAIL: shell never up"; tail -20 "$SHELL_LOG"; exit 1; }
  gnoblin_wait_for_log "$SHELL_LOG" "SCRIPT version=A" 10 || true

  gnoblin() { gdbus call --session --dest org.gnoblin.Shell --object-path /org/gnoblin/Shell \
               --method "org.gnoblin.Shell.$@" 2>&1; }

  rc=0
  if grep -q "SCRIPT version=A" "$SHELL_LOG"; then echo "  ok: script loaded (version=A)"; else echo "  FAIL: script did not load"; grep -i script "$SHELL_LOG" | tail; rc=1; fi
  echo "ListScripts -> $(gnoblin ListScripts)"
  case "$(gnoblin ListScripts)" in *hello.js*) echo "  ok: ListScripts shows hello.js";; *) echo "  FAIL: not listed"; rc=1;; esac

  printf "export default (api) => { api.log(\"SCRIPT version=B\"); };\n" > "$SCRIPTDIR/hello.js"
  reload="$(gnoblin ReloadScripts)"
  echo "ReloadScripts -> $reload"
  if grep -q "SCRIPT version=B" "$SHELL_LOG"; then
    echo "  ok: ReloadScripts waited for version=B to load"
  else
    echo "  FAIL: ReloadScripts replied before script load completed"; grep "SCRIPT version" "$SHELL_LOG"; rc=1
  fi

  printf "this is not valid JavaScript\n" > "$SCRIPTDIR/hello.js"
  if reload_error="$(gnoblin ReloadScripts)"; then
    echo "  FAIL: invalid script reload reported success"; rc=1
  else
    case "$reload_error" in
      *ReloadFailed*) echo "  ok: invalid script reload returned a D-Bus error" ;;
      *) echo "  FAIL: invalid script reload returned wrong error: $reload_error"; rc=1 ;;
    esac
  fi

  kill $SHELL_PID 2>/dev/null || true
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (user scripting + hot-reload)" || echo ">> RESULT: FAIL (rc=$rc). log -> $LAST_LOG"
exit "$rc"
