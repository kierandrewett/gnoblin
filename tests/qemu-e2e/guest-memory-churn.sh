#!/usr/bin/env bash
# Guest: the compositor does not leak memory while windows open, take focus, move and close (GitHub #115).
#
# Records the resident memory of the compositor and the Lua worker, then runs rounds of window churn: two GTK windows
# open and take focus, move across the screen with gnoblinctl, and close. (gnoblinctl cannot move focus itself, so new
# windows do it.) Window changes make the compositor publish window snapshots to the control plane, which is where the
# leak in #115 grew at 8 to 9 MB/s. Resident memory moves by about 60 KB per extra window and stays at its high-water
# mark, so a real leak shows up within one round. The run fails if resident memory grows by more than
# GNOBLIN_TEST_MAX_GROWTH_KB (default 40000) in either process over GNOBLIN_TEST_ROUNDS rounds (default 12).
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
ROUNDS="${GNOBLIN_TEST_ROUNDS:-12}"
MAX_GROWTH_KB="${GNOBLIN_TEST_MAX_GROWTH_KB:-40000}"
fail=0

cat >/tmp/churn-app.py <<'PY'
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
loop = GLib.MainLoop()
window = Gtk.Window(title=sys.argv[1])
window.set_default_size(300, 200)
window.set_child(Gtk.Label(label=sys.argv[1]))
window.present()
GLib.timeout_add_seconds(int(sys.argv[2]), lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY

# rss_kb ROLE reads the resident memory of one Gnoblin process. The guardian, the compositor, the supervisor and the Lua
# worker all run the gnoblin binary, so the process is picked by its command line.
rss_kb() {
    local pid
    for pid in $(pgrep -x gnoblin); do
        if tr '\0' ' ' <"/proc/$pid/cmdline" | grep -q -e "$1"; then
            awk '/^VmRSS:/ {print $2}' "/proc/$pid/status"
            return
        fi
    done
    echo 0
}
id_of() {
    "$G" window list | python3 -c '
import json, sys
ids = [w["id"] for w in json.load(sys.stdin)["windows"] if w["title"] == "'"$1"'"]
print(ids[0] if ids else "")'
}

# One warm-up round lets caches fill before the first reading.
churn() {
    nohup env GDK_BACKEND=wayland python3 /tmp/churn-app.py churn-a 6 >/dev/null 2>&1 </dev/null &
    nohup env GDK_BACKEND=wayland python3 /tmp/churn-app.py churn-b 6 >/dev/null 2>&1 </dev/null &
    sleep 2
    a="$(id_of churn-a)"
    b="$(id_of churn-b)"
    for step in 1 2 3 4 5 6 7 8; do
        x=$((step * 60))
        [ -n "$a" ] && "$G" window move "$a" "$x" 100 >/dev/null 2>&1
        [ -n "$b" ] && "$G" window move "$b" $((700 - x)) 300 >/dev/null 2>&1
    done
    sleep 5
}

churn
comp_before="$(rss_kb '--wayland')"
worker_before="$(rss_kb '--internal-runtime-worker')"
echo "after warm-up: compositor ${comp_before} KB, Lua worker ${worker_before} KB"
if [ "$comp_before" -eq 0 ] || [ "$worker_before" -eq 0 ]; then
    echo "FAIL a Gnoblin process was not found (compositor ${comp_before}, worker ${worker_before})"
    exit 1
fi
for round in $(seq 1 "$ROUNDS"); do
    churn
    echo "round $round: compositor $(rss_kb '--wayland') KB, Lua worker $(rss_kb '--internal-runtime-worker') KB"
done
comp_after="$(rss_kb '--wayland')"
worker_after="$(rss_kb '--internal-runtime-worker')"
echo "after $ROUNDS rounds: compositor ${comp_after} KB, Lua worker ${worker_after} KB (limit ${MAX_GROWTH_KB} KB growth)"
check_growth() {
    # check_growth NAME BEFORE AFTER
    local growth=$(($3 - $2))
    if [ "$growth" -le "$MAX_GROWTH_KB" ]; then
        echo "PASS the $1 memory stays flat while windows open, take focus, move and close (growth ${growth} KB)"
    else
        echo "FAIL the $1 memory grew by ${growth} KB over $ROUNDS rounds"
        fail=$((fail + 1))
    fi
}
check_growth compositor "$comp_before" "$comp_after"
check_growth "Lua worker" "$worker_before" "$worker_after"
echo "failures: $fail"
exit "$fail"
