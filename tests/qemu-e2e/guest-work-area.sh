#!/usr/bin/env bash
# Guest: each monitor record carries a work_area, and the monitor-changed event reports it.
#
# Without a panel the work area is the whole monitor. With Waybar (a 30 px exclusive zone on top) it loses the top
# strip. When Waybar closes it is the whole monitor again. The Lua listener in configs/99-test-workarea.lua logs the
# changed list of every gnoblin.monitor.changed event.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
LISTENER="$HOME/.config/gnoblin/config/99-test-workarea.lua"
fail=0

check() {
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: $2, expected: $3)"
        fail=$((fail + 1))
    fi
}

# Prints "OK" when every monitor's work_area equals the monitor rectangle with TOP pixels removed from the top.
areas() {
    "$G" monitor list | python3 -c '
import json, sys
top = int(sys.argv[1])
bad = []
for m in json.load(sys.stdin)["monitors"]:
    a = m.get("work_area")
    want = (m["x"], m["y"] + top, m["width"], m["height"] - top)
    if not a or (a["x"], a["y"], a["width"], a["height"]) != want:
        bad.append("%s %s != %s" % (m["id"], a, want))
print("OK" if not bad else "; ".join(bad))' "$1"
}

cp /tmp/99-test-workarea.lua "$LISTENER"
"$G" config reload >/dev/null
sleep 3
: > /tmp/workarea-events.log

check "no panel: the work area is the whole monitor" "$(areas 0)" "OK"
nohup waybar >/dev/null 2>&1 < /dev/null &
sleep 5
check "Waybar running: the work area loses its 30 px top strip" "$(areas 30)" "OK"
events_with_panel="$(cat /tmp/workarea-events.log)"
pkill -x waybar
sleep 3
check "Waybar closed: the work area is the whole monitor again" "$(areas 0)" "OK"

case "$events_with_panel" in
    *work_area*) echo "PASS gnoblin.monitor.changed reported work_area when the panel appeared" ;;
    *) echo "FAIL no monitor-changed event listed work_area (log: $events_with_panel)"; fail=$((fail + 1)) ;;
esac

rm -f "$LISTENER"
"$G" config reload >/dev/null
echo "failures: $fail"
exit "$fail"
