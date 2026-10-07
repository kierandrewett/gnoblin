#!/usr/bin/env bash
# Guest: tell a real leak from allocator retention. Churn 120 windows, then call malloc_trim(0) in the compositor.
# A real leak keeps its RSS after the trim. Allocator retention drops back. Diagnostic only (needs gdb and sudo).
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)" WAYLAND_DISPLAY=wayland-0 DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
pid="$(pgrep -f 'gnoblin --wayland' | head -1)"
rss() { awk '/VmRSS/ {printf "%d", $2 / 1024}' /proc/"$pid"/status; }
churn() {
    for c in $(seq 1 60); do
        nohup foot -T "t-a-$c" >/dev/null 2>&1 </dev/null & a=$!
        sleep 0.4
        nohup foot -T "t-b-$c" >/dev/null 2>&1 </dev/null & b=$!
        sleep 0.4
        kill $a $b 2>/dev/null
        sleep 0.5
    done
}
nohup swaybg -c "#223344" >/dev/null 2>&1 </dev/null &
sleep 1
echo "start: $(rss) MB"
churn
sleep 3
echo "after 120 windows: $(rss) MB"
sudo gdb -p "$pid" -batch -ex 'call (int)malloc_trim(0)' >/tmp/trim.out 2>&1
sleep 2
echo "after malloc_trim(0): $(rss) MB ($(grep -o '\$1 = [0-9]*' /tmp/trim.out))"
pkill -x swaybg; true
