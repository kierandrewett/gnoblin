#!/usr/bin/env bash
# Guest: helper for run-config-fallback.sh. Usage: guest-config-fallback.sh install | read | clean
#
# install appends a Lua syntax error to the root init.lua, after saving a copy. A broken root file cannot be salvaged,
# so the session must fall back to the last good configuration. (A single invalid setting, or a broken included file, is
# ignored instead; see run-partial-recovery.sh.) read reports the session state after a restart. clean restores the
# saved init.lua.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/init.lua"
SAVED="$HOME/.config/gnoblin/init.lua.fallback-test-saved"

case "${1:-}" in
    install)
        cp -p "$F" "$SAVED"
        printf '\ngnoblin.configure {cursor = {size = 55}\n' >> "$F"
        ;;
    read)
        echo "compositors=$(pgrep -x gnoblin | wc -l)"
        echo "status=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        echo "marker=$(head -1 "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null)"
        echo "marker_error=$(sed -n 2p "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null)"
        echo "log=$(sudo journalctl -b --no-pager 2>/dev/null | grep 'gnoblin: configuration fell back to' | tail -1 | sed 's/.*gnoblin: //')"
        ;;
    clean)
        if [ -f "$SAVED" ]; then mv -f "$SAVED" "$F"; fi
        ;;
    *)
        echo "usage: $0 install | read | clean" >&2
        exit 2
        ;;
esac
