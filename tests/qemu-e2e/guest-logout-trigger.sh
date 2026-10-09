#!/usr/bin/env bash
# Guest: ask the running Gnoblin session to log out. The connection drops as the session ends.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
mark="$(date '+%Y-%m-%d %H:%M:%S')"
echo "$mark" > /tmp/logout-mark
"${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX}/bin/gnoblinctl" logout 2>&1 | tail -1
