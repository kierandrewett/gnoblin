#!/usr/bin/env bash
# Guest: bind a dynamic shortcut from Lua.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
: > /tmp/shortcut-events.log
cp /tmp/99-test-shortcut.lua "$HOME/.config/gnoblin/config/99-test-shortcut.lua"
"$G" config reload | tail -1
sleep 3
cat /tmp/shortcut-events.log
