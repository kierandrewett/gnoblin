#!/usr/bin/env bash
# Guest: read the activation log, then disable the test config.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
echo "== events"
cat /tmp/shortcut-events.log
printf -- '-- disabled\n' > "$HOME/.config/gnoblin/config/99-test-shortcut.lua"
"$G" config reload | tail -1
echo "alive: $(pgrep -x gnoblin | wc -l)"
