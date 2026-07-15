#!/usr/bin/env bash
# Prove the `notifications` feature toggle: gnoblin owns org.freedesktop.Notifications
# by default, and DISABLING the feature releases the bus name so an external daemon
# (e.g. a quickshell notification service) can own it — live, no restart.
#
# Uses the keyfile gsettings backend so the shell and the (separate-process) fdo
# notification daemon share the org.gnoblin.shell key, as dconf does on a real session.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GDK_BACKEND=wayland GNOME_SHELL_SESSION_MODE=gnoblin XDG_CURRENT_DESKTOP=GNOME:Gnoblin

DK="$(mktemp -d /tmp/gnoblin-nt.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GTK_A11Y=none NO_AT_BRIDGE=1
# dconf backend: cross-process change notification via the dconf D-Bus service, so
# the (separate-process) fdo daemon sees the shell's SetFeature — as on a real
# session. (memory is per-process; keyfile needs a file monitor the sandbox lacks.)
# The dconf DB is isolated under XDG_CONFIG_HOME/dconf/user.
export GSETTINGS_BACKEND=dconf
export DISP="gnoblin-nt-$$" GS="$SHELL_BIN"

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1
export DK

dbus-run-session --config-file="$CONF" -- bash -uo pipefail -c '
  source "$ROOT/scripts/gnoblin-test-lib.sh"
  "$GS" --headless --wayland --no-x11 --mode=gnoblin --virtual-monitor 1280x800 \
    --wayland-display "$DISP" >"$DK/shell.log" 2>&1 &
  gdbus wait --session --timeout 30 org.gnoblin.Shell || { echo "FAIL: shell never up"; exit 1; }

  fdo=org.freedesktop.Notifications
  owned() { gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
              --method org.freedesktop.DBus.NameHasOwner "$fdo" | tr -d "\n"; }
  ctl() { gdbus call --session --dest org.gnoblin.Shell --object-path /org/gnoblin/Shell \
            --method "org.gnoblin.Shell.$@" >/dev/null; }
  name_is_owned() { case "$(owned)" in *true*) return 0;; *) return 1;; esac; }
  name_is_unowned() { ! name_is_owned; }

  # Start the fdo notification daemon (it owns org.freedesktop.Notifications).
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.StartServiceByName org.gnome.Shell.Notifications 0 >/dev/null 2>&1
  gnoblin_wait_until 10 name_is_owned || true

  rc=0
  d="$(owned)";  echo "default          -> $d"
  case "$d" in *true*)  echo "  ok: gnome owns org.freedesktop.Notifications by default";; *) echo "  FAIL: not owned by default"; rc=1;; esac

  ctl SetFeature notifications false
  gnoblin_wait_until 10 name_is_unowned || true
  off="$(owned)"; echo "notifications off -> $off"
  case "$off" in *false*) echo "  ok: name RELEASED (an external daemon can own it now)";; *) echo "  FAIL: name not released"; rc=1;; esac

  ctl SetFeature notifications true
  gnoblin_wait_until 10 name_is_owned || true
  on="$(owned)";  echo "notifications on  -> $on"
  case "$on" in *true*)  echo "  ok: name RECLAIMED";; *) echo "  FAIL: name not reclaimed"; rc=1;; esac

  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (notifications ownership toggle)" || echo ">> RESULT: FAIL (rc=$rc)"
exit "$rc"
