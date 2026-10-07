#!/usr/bin/env bash
# Guest: the compositor's memory must stay flat while windows open, close and change focus.
# Regression check for the event conversion leak (about 8 MB/s) that froze real sessions.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
SECONDS_TO_RUN="${GNOBLIN_MEMORY_SECONDS:-90}"
LIMIT_MB="${GNOBLIN_MEMORY_LIMIT_MB:-40}"

pid="$(pgrep -f 'gnoblin --wayland' | head -1)"
[ -n "$pid" ] || { echo "FAIL no compositor process"; exit 1; }
rss_mb() { awk '/VmRSS/ {printf "%d", $2 / 1024}' /proc/"$pid"/status; }
swap_mb() { awk '/VmSwap/ {printf "%d", $2 / 1024}' /proc/"$pid"/status; }

nohup swaybg -c "#223344" >/dev/null 2>&1 < /dev/null &
sleep 2
before="$(rss_mb)"
echo "compositor pid $pid: rss ${before} MB, swap $(swap_mb) MB before"

end=$((SECONDS + SECONDS_TO_RUN))
cycle=0
while [ "$SECONDS" -lt "$end" ]; do
    cycle=$((cycle + 1))
    nohup foot -T "mem-a-$cycle" >/dev/null 2>&1 < /dev/null &
    first=$!
    sleep 0.4
    nohup foot -T "mem-b-$cycle" >/dev/null 2>&1 < /dev/null &
    second=$!
    sleep 0.4
    "$G" window list >/dev/null 2>&1
    "$G" workspace list >/dev/null 2>&1
    # change focus and stacking: click the strip of the lower window that the upper one leaves uncovered
    "$G" window list 2>&1 | python3 -c '
import json, sys
windows = [w for w in json.load(sys.stdin)["windows"] if w["title"].startswith("mem-")]
if windows:
    f = windows[-1]["frame"]
    print(f["x"] + 20, f["y"] + 300)' > /tmp/mem-click.txt
    if [ "${GNOBLIN_MEMORY_CLICKS:-1}" = 1 ] && [ -s /tmp/mem-click.txt ]; then
        python3 - $(cat /tmp/mem-click.txt) <<'PY' 2>/dev/null
import sys
import time
from gi.repository import Gio, GLib
x, y = float(sys.argv[1]), float(sys.argv[2])
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
def call(path, iface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, iface, method, args, None, Gio.DBusCallFlags.NONE, 5000, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"
session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (x, y)))
call(session, iface, "NotifyPointerButton", GLib.Variant("(ib)", (0x110, True)))
call(session, iface, "NotifyPointerButton", GLib.Variant("(ib)", (0x110, False)))
time.sleep(0.2)
call(session, iface, "Stop", None)
PY
    fi
    kill "$first" "$second" 2>/dev/null
    sleep 0.5
done

after="$(rss_mb)"
growth=$((after - before))
echo "after $cycle cycles in ${SECONDS_TO_RUN}s: rss ${after} MB, swap $(swap_mb) MB, growth ${growth} MB"
pkill -x swaybg
if [ "$growth" -lt "$LIMIT_MB" ]; then
    echo "PASS compositor memory stays flat under window churn (growth ${growth} MB, limit ${LIMIT_MB} MB)"
else
    echo "FAIL compositor memory grows under window churn (growth ${growth} MB, limit ${LIMIT_MB} MB)"
fi
