#!/usr/bin/env bash
# Guest: attach heaptrack to the compositor, churn windows, then print the allocation sites still holding memory.
# Diagnostic only. Attaching slows the compositor down, so the number of cycles is small.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CYCLES="${GNOBLIN_HEAPTRACK_CYCLES:-60}"

command -v heaptrack >/dev/null || sudo dnf install -y heaptrack >/tmp/heaptrack-install.log 2>&1
command -v heaptrack >/dev/null || { echo "heaptrack is not available"; tail -3 /tmp/heaptrack-install.log; exit 1; }

pid="$(pgrep -f 'gnoblin --wayland' | head -1)"
echo "compositor pid $pid, rss $(awk '/VmRSS/ {printf "%d", $2 / 1024}' /proc/"$pid"/status) MB"
nohup swaybg -c "#223344" >/dev/null 2>&1 < /dev/null &
sleep 1

# heaptrack ignores -o when it attaches to a running process and writes ~/heaptrack.<name>.<pid>.zst.
rm -f "$HOME"/heaptrack.gnoblin.*.zst
nohup heaptrack --pid "$pid" >/tmp/heaptrack-run.log 2>&1 < /dev/null &
sleep 8
cat /tmp/heaptrack-run.log | head -5

for cycle in $(seq 1 "$CYCLES"); do
    nohup foot -T "ht-a-$cycle" >/dev/null 2>&1 < /dev/null &
    first=$!
    sleep 0.5
    nohup foot -T "ht-b-$cycle" >/dev/null 2>&1 < /dev/null &
    second=$!
    sleep 0.5
    "$G" window list >/dev/null 2>&1
    kill "$first" "$second" 2>/dev/null
    sleep 0.7
done
pkill -x swaybg
sleep 3
pkill -INT -f "heaptrack --pid" 
sleep 6
data="$(ls -t "$HOME"/heaptrack.gnoblin.*.zst 2>/dev/null | head -1)"
echo "data file: $data"
if [ -n "$data" ]; then
    echo "== top leaks (allocation sites still holding memory)"
    heaptrack_print -f "$data" --print-leaks 1 --print-histogram 0 -n 12 2>&1 | sed -n '/MEMORY LEAKS/,/MOST MEMORY ALLOCATED/p' | head -90 | cut -c1-160
fi
echo "alive: $(pgrep -f 'gnoblin --wayland' | wc -l)"
