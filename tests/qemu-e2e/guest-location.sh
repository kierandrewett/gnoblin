#!/usr/bin/env bash
# Guest: ask GeoClue for location while the demo agent runs, then after it is stopped.
# Needs /etc/geoclue/conf.d/90-gnoblin-test.conf with: [agent] whitelist=gnome-shell;...;gnoblin
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-location.lua"

gsettings set org.gnome.system.location enabled true
cp /tmp/99-test-location.lua "$CONFIG"
"$G" config reload | tail -1
sleep 6
: > /tmp/location-events.log

ask() {
    timeout 60 python3 /tmp/geoclue-ask.py 2>&1 | tail -2
    sleep 3
    echo "handler events: [$(cat /tmp/location-events.log)]"
}

echo "== demo agent processes: $(pgrep -f 'geoclue-2.0/demos/agent' | wc -l)"
echo "== ask while the demo agent runs"
ask
echo "== stop the demo agent, then ask again"
pkill -f 'geoclue-2.0/demos/agent'
sleep 2
echo "demo agent processes: $(pgrep -f 'geoclue-2.0/demos/agent' | wc -l)"
ask

printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload | tail -1
gsettings reset org.gnome.system.location enabled
echo "alive: $(pgrep -x gnoblin | wc -l)"
