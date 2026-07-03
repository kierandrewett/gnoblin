#!/usr/bin/env bash
# Prove live extension hot-reload: install a test extension that logs a version
# string on enable, enable it, edit its code, ReloadExtension over org.gnoblin.Shell,
# and confirm the NEW code ran — all without restarting the compositor.
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
export GDK_BACKEND=wayland GNOME_SHELL_SESSION_MODE=gnoblin XDG_CURRENT_DESKTOP=GNOME:Gnoblin

DK="$(mktemp -d /tmp/gnoblin-hr.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home,sys}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
# Put the test extension on the SYSTEM data path so it's always scanned (no
# allow-extension-installation gate that user-dir extensions need).
export XDG_DATA_DIRS="$DK/sys:$XDG_DATA_DIRS"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GSETTINGS_BACKEND=memory GTK_A11Y=none NO_AT_BRIDGE=1
export DISP="gnoblin-hr-$$" SHELL_LOG="$DK/shell.log"

# Install a test extension (system-path) at version A.
EXTDIR="$DK/sys/gnome-shell/extensions/hrtest@gnoblin"
mkdir -p "$EXTDIR"
cat > "$EXTDIR/metadata.json" <<JSON
{ "uuid": "hrtest@gnoblin", "name": "HR Test", "description": "hot-reload probe",
  "shell-version": ["49"], "session-modes": ["gnoblin", "user"] }
JSON
write_ext() { cat > "$EXTDIR/extension.js" <<JS
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
export default class extends Extension {
    enable()  { console.log('HRTEST version=$1'); }
    disable() {}
}
JS
}
write_ext A
export EXTDIR

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  cp "$SHELL_LOG" /tmp/gnoblin-hot-reload-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

dbus-run-session --config-file="$CONF" -- bash -uo pipefail -c '
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor 1280x800 --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  gdbus wait --session --timeout 30 org.gnoblin.Shell || { echo "FAIL: shell never up"; tail -20 "$SHELL_LOG"; exit 1; }

  gshell() { gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
               --method "org.gnome.Shell.Extensions.$@" 2>&1; }
  gnoblin() { gdbus call --session --dest org.gnoblin.Shell --object-path /org/gnoblin/Shell \
               --method "org.gnoblin.Shell.$@" 2>&1; }

  rc=0
  echo "enable -> $(gshell EnableExtension hrtest@gnoblin)"
  sleep 1
  if grep -q "HRTEST version=A" "$SHELL_LOG"; then echo "  ok: extension loaded (version=A)"; else echo "  FAIL: extension did not load"; tail -15 "$SHELL_LOG"; rc=1; fi

  echo "ListExtensions -> $(gnoblin ListExtensions)"
  case "$(gnoblin ListExtensions)" in *hrtest@gnoblin*) echo "  ok: ListExtensions shows it";; *) echo "  FAIL: not listed"; rc=1;; esac

  # Edit the extension code to version B, then hot-reload it.
  cat > "$EXTDIR/extension.js" <<JS
import {Extension} from '"'"'resource:///org/gnome/shell/extensions/extension.js'"'"';
export default class extends Extension {
    enable()  { console.log('"'"'HRTEST version=B'"'"'); }
    disable() {}
}
JS
  echo "ReloadExtension -> $(gnoblin ReloadExtension hrtest@gnoblin)"
  sleep 1
  if grep -q "HRTEST version=B" "$SHELL_LOG"; then echo "  ok: HOT-RELOAD picked up new code (version=B)"; else echo "  FAIL: code not reloaded"; grep HRTEST "$SHELL_LOG"; rc=1; fi

  kill $SHELL_PID 2>/dev/null || true
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (live extension hot-reload)" || echo ">> RESULT: FAIL (rc=$rc). log -> /tmp/gnoblin-hot-reload-last.log"
exit "$rc"
