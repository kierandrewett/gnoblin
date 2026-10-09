#!/usr/bin/env bash
# Guest: a window on a monitor that goes away is not lost.
#
# A window is placed on the second virtual monitor. The monitor is then disabled with gdctl, as an unplug would. The
# window must move to the remaining monitor and lie fully inside it, the compositor must keep running, and the window
# must stay reachable when the monitor comes back. Where the window ends up after the monitor returns is Mutter's
# choice, so it is reported and not asserted. Both monitors are restored at the end.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
GD="${GNOBLIN_PREFIX}/bin/gdctl"
fail=0

restore_monitors() {
    "$GD" set --logical-monitor --primary --monitor Virtual-1 --scale 1 \
        --logical-monitor --monitor Virtual-2 --scale 1 --right-of Virtual-1 >/dev/null 2>&1
}
trap 'pkill -f "foot -T unplug-test" 2>/dev/null; restore_monitors' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

restore_monitors
sleep 3
read -r mon_w mon_h <<<"$("$G" monitor list | python3 -c '
import json, sys
data = json.load(sys.stdin)
monitors = data.get("monitors", data) if isinstance(data, dict) else data
m = monitors[0]
box = m.get("rect") or m.get("geometry") or m
print(m.get("width", box.get("width")), m.get("height", box.get("height")))')"

# Prints "MONITOR X Y W H" for the test window, or "gone".
state() {
    "$G" window list | python3 -c '
import json, sys
found = [w for w in json.load(sys.stdin)["windows"] if w["title"] == "unplug-test"]
if not found:
    print("gone")
else:
    w = found[0]; f = w["frame"]
    print("%s %d %d %d %d" % (w["monitor_id"], f["x"], f["y"], f["width"], f["height"]))'
}
window_id() {
    "$G" window list | python3 -c '
import json, sys
print([w["id"] for w in json.load(sys.stdin)["windows"] if w["title"] == "unplug-test"][0])'
}

nohup foot -T unplug-test >/dev/null 2>&1 < /dev/null &
sleep 4
"$G" window move "$(window_id)" $((mon_w + 220)) 200 >/dev/null 2>&1
sleep 2
read -r m0 x0 y0 w0 h0 <<<"$(state)"
check "the window is on the second monitor before the unplug" "$m0" "Virtual-2"

"$GD" set --logical-monitor --primary --monitor Virtual-1 --scale 1 >/dev/null 2>&1
sleep 3
read -r m1 x1 y1 w1 h1 <<<"$(state)"
check "the window is still there after the monitor is disabled" "$([ "$m1" != gone ] && echo present || echo gone)" "present"
check "the window moved to the remaining monitor" "$m1" "Virtual-1"
inside="no"
if [ "$m1" != gone ] && [ "$x1" -ge 0 ] && [ "$y1" -ge 0 ] && [ $((x1 + w1)) -le "$mon_w" ] && [ $((y1 + h1)) -le "$mon_h" ]; then inside="yes"; fi
check "the window lies fully inside the remaining monitor" "$inside" "yes"
check "the size is kept" "$w1 $h1" "$w0 $h0"
check "the compositor keeps running" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

restore_monitors
sleep 3
read -r m2 x2 y2 w2 h2 <<<"$(state)"
echo "info after the monitor returns: $m2 $x2 $y2 ${w2:-} ${h2:-}"
check "the window is still reachable after the monitor returns" "$([ "$m2" != gone ] && echo present || echo gone)" "present"
check "the compositor still runs after the monitor returns" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

echo "failures: $fail"
exit "$fail"
