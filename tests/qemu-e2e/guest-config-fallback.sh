#!/usr/bin/env bash
# Guest: helper for run-config-fallback.sh. Usage: guest-config-fallback.sh install | read | clean
#
# install writes a config with one invalid keybinding and one valid setting. read reports the session state after a
# restart. clean removes the test file.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-config-fallback.lua"

case "${1:-}" in
    install)
        printf 'gnoblin.configure {keybindings = {wm = {panel_run_dialog = {"<Super>r"}}}, cursor = {size = 55}}\n' > "$F"
        ;;
    read)
        echo "compositors=$(pgrep -x gnoblin | wc -l)"
        echo "status=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        echo "marker=$(head -1 "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null)"
        echo "marker_error=$(sed -n 2p "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null)"
        echo "log=$(sudo journalctl -b --no-pager 2>/dev/null | grep 'gnoblin: configuration fell back to' | tail -1 | sed 's/.*gnoblin: //')"
        ;;
    clean)
        rm -f "$F"
        ;;
    *)
        echo "usage: $0 install | read | clean" >&2
        exit 2
        ;;
esac
