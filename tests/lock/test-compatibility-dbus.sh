#!/usr/bin/env bash
# Verify compatibility names on a bus with no host ScreenSaver activation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d /tmp/gnoblin-lock-dbus.XXXXXX)"
export WORK
trap 'rm -rf "$WORK"' EXIT
CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$WORK" "$ROOT")"

dbus-run-session --config-file="$CONF" -- bash -euo pipefail -c '
  python3 -u - <<"PY" >"$WORK/broker.log" 2>&1 &
import importlib.machinery
import sys
sys.path.insert(0, "src/lock")
module = importlib.machinery.SourceFileLoader("gnoblin_lockd", "src/lock/gnoblin-lockd.py").load_module()
broker = module.LockBroker(module.Config("/dev/null"))
broker.config.own_compatibility_names = True
from gi.repository import Gio, GLib
Gio.bus_own_name(Gio.BusType.SESSION, module.BUS_NAME, Gio.BusNameOwnerFlags.NONE,
                 broker.bus_acquired, None, None)
def set_native_ready():
    broker._apply_native_properties({
        "State": GLib.Variant("s", "unlocked"),
        "Active": GLib.Variant("b", False),
        "Capability": GLib.Variant("u", 1),
        "LauncherReady": GLib.Variant("b", True),
    })
    return GLib.SOURCE_REMOVE
GLib.timeout_add(100, set_native_ready)
GLib.MainLoop().run()
PY
  child=$!
  for _ in $(seq 1 30); do
    if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.GetNameOwner org.gnome.ScreenSaver >"$WORK/screen-owner" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  test -s "$WORK/screen-owner"
  gdbus call --session --dest org.gnome.ScreenSaver \
    --object-path /org/gnome/ScreenSaver --method org.gnome.ScreenSaver.GetActive >"$WORK/active"
  test "$(<"$WORK/active")" = "(false,)"
  test "$(gdbus call --session --dest org.gnoblin.Lock --object-path /org/gnoblin/Lock \
    --method org.freedesktop.DBus.Properties.Get org.gnoblin.Lock CompatibilityReady)" = "(<true>,)"
  for name in org.gnoblin.Lock org.gnome.ScreenSaver org.gnome.Shell.ScreenShield; do
    gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.GetNameOwner "$name"
  done | sed -E "s/.*'"'"'(:[0-9.]+)'"'"'.*/\1/" | sort -u >"$WORK/owners"
  test "$(wc -l <"$WORK/owners")" -eq 1
  kill "$child"
  wait "$child" 2>/dev/null || true
'
