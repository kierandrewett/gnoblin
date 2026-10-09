#!/usr/bin/env bash
# Guest: a layer-shell bar on a monitor that goes away is closed cleanly, and its client can put it back.
#
# waybar is started with a 30 pixel bar pinned to the second virtual monitor. The compositor must list it as a top layer
# surface on that monitor with an exclusive zone of 30. The monitor is then disabled with gdctl: the surface must go,
# the compositor and the waybar process must keep running. When the monitor returns, waybar must create its bar there
# again. Both monitors are restored at the end.
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
trap 'pkill -x waybar 2>/dev/null; restore_monitors; rm -f /tmp/waybar-test.json /tmp/waybar-test.css' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat > /tmp/waybar-test.json <<'JSON'
{"layer": "top", "position": "top", "height": 30, "output": ["Virtual-2"], "name": "probebar",
 "modules-left": [], "modules-center": [], "modules-right": []}
JSON
echo '* { background: #cc3300; }' > /tmp/waybar-test.css

# Prints "NAMESPACE MONITOR LAYER ZONE" for each layer surface, or "none".
layers() {
    "$G" layer list 2>&1 | python3 -c '
import json, sys
data = json.load(sys.stdin)
items = data.get("layers", data.get("surfaces", data))
rows = ["%s %s %s %s" % (l.get("namespace"), l.get("monitor_id") or l.get("monitor"), l.get("layer"), l.get("exclusive_zone"))
        for l in items] if isinstance(items, list) else []
print(";".join(rows) or "none")'
}
running() { "$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"'; }

restore_monitors
sleep 3
nohup waybar -c /tmp/waybar-test.json -s /tmp/waybar-test.css >/tmp/waybar.log 2>&1 < /dev/null &
sleep 5
check "the bar is a top layer surface on the second monitor with an exclusive zone of 30" "$(layers)" "waybar Virtual-2 top 30"

"$GD" set --logical-monitor --primary --monitor Virtual-1 --scale 1 >/dev/null 2>&1
sleep 3
check "the bar's surface is gone after its monitor is disabled" "$(layers)" "none"
check "the compositor keeps running after the monitor is disabled" "$(running)" '"state":"running"'
check "the waybar process keeps running after its surface is closed" "$(pgrep -x waybar | wc -l)" "1"

restore_monitors
sleep 5
check "waybar puts its bar back on the monitor when it returns" "$(layers)" "waybar Virtual-2 top 30"
check "the compositor still runs after the monitor returns" "$(running)" '"state":"running"'

echo "failures: $fail"
exit "$fail"
