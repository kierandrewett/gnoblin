#!/usr/bin/env bash
# Boot patched gnome-shell (gnoblin mode) headless and exercise the org.gnoblin.*
# control protocol end-to-end over D-Bus:
#   - org.gnoblin.Shell.Ping        -> "pong"
#   - org.gnoblin.Shell.GetVersion  -> "*-gnoblin"
#   - org.gnoblin.Shell.Reload      -> triggers a soft in-process reload (log check)
#
# gnome-shell AND the gdbus calls run inside one dbus-run-session, so they share
# the same isolated session bus (no host bus leakage, no address plumbing).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
MONITOR="${MONITOR:-1280x800}"

[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GDK_BACKEND=wayland GNOME_SHELL_SESSION_MODE=gnoblin XDG_CURRENT_DESKTOP=GNOME:Gnoblin

DK="$(mktemp -d /tmp/gnoblin-dbus.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GSETTINGS_BACKEND=memory GTK_A11Y=none NO_AT_BRIDGE=1
export DISP="gnoblin-dbus-$$" SHELL_LOG="$DK/shell.log"

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  cp "$SHELL_LOG" /tmp/gnoblin-dbus-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

# Everything below shares the one dbus-run-session bus.
dbus-run-session --config-file="$CONF" -- bash -euo pipefail -c '
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor "'"$MONITOR"'" --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!

  # Wait for the shell to own org.gnoblin.Shell (implies started + component up).
  if ! gdbus wait --session --timeout 30 org.gnoblin.Shell; then
    echo "FAIL: org.gnoblin.Shell never appeared"; tail -20 "$SHELL_LOG"; kill $SHELL_PID 2>/dev/null; exit 1
  fi

  rc=0
  call() { gdbus call --session --dest org.gnoblin.Shell \
             --object-path /org/gnoblin/Shell --method "org.gnoblin.Shell.$1" 2>&1; }

  callp() { gdbus call --session --dest org.gnoblin.Shell \
              --object-path /org/gnoblin/Shell --method "org.gnoblin.Shell.$@" 2>&1; }

  ping="$(call Ping)";        echo "Ping       -> $ping"
  ver="$(call GetVersion)";   echo "GetVersion -> $ver"
  reload="$(call Reload)";    echo "Reload     -> $reload"

  case "$ping"   in *pong*)     echo "  ok: Ping";;        *) echo "  FAIL: Ping"; rc=1;; esac
  case "$ver"    in *-gnoblin*) echo "  ok: GetVersion";;  *) echo "  FAIL: GetVersion"; rc=1;; esac
  # Reload is void; assert the soft-reload actually ran from the shell log.
  sleep 1
  if grep -q "gnoblin: soft-reload" "$SHELL_LOG"; then echo "  ok: Reload (soft-reload ran)"; else echo "  FAIL: Reload (no soft-reload log)"; rc=1; fi

  # --- feature toggles ---
  feats="$(call ListFeatures)"; echo "ListFeatures -> $feats"
  case "$feats" in *osd*screenshot*|*screenshot*osd*) echo "  ok: ListFeatures (osd + screenshot)";; *) echo "  FAIL: ListFeatures"; rc=1;; esac

  g0="$(callp GetFeature osd)";                 echo "GetFeature osd (default) -> $g0"
  case "$g0" in *true*)  echo "  ok: osd default enabled";; *) echo "  FAIL: osd default"; rc=1;; esac

  callp SetFeature osd false >/dev/null
  g1="$(callp GetFeature osd)";                 echo "GetFeature osd (after off) -> $g1"
  case "$g1" in *false*) echo "  ok: SetFeature osd off";; *) echo "  FAIL: SetFeature off"; rc=1;; esac

  callp SetFeature osd true >/dev/null
  g2="$(callp GetFeature osd)";                 echo "GetFeature osd (after on) -> $g2"
  case "$g2" in *true*)  echo "  ok: SetFeature osd on";; *) echo "  FAIL: SetFeature on"; rc=1;; esac

  gu="$(callp GetFeature bogus)";               echo "GetFeature bogus -> $gu"
  case "$gu" in *false*) echo "  ok: unknown feature -> false";; *) echo "  FAIL: unknown feature"; rc=1;; esac

  kill $SHELL_PID 2>/dev/null || true
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (org.gnoblin.* round-trip)" || echo ">> RESULT: FAIL (rc=$rc). log -> /tmp/gnoblin-dbus-last.log"
exit "$rc"
