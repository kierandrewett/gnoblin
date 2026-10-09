#!/usr/bin/env bash
# Guest: helper for run-partial-recovery.sh. Usage: guest-partial-recovery.sh install | read | clean
#
# install writes a config with one invalid keybinding and one valid setting, and a second file with a Lua syntax error.
# read reports the session state after a restart. clean removes the test files.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-partial-recovery.lua"
B="$HOME/.config/gnoblin/config/98-test-partial-recovery-syntax.lua"

case "${1:-}" in
    install)
        sudo journalctl -b --no-pager 2>/dev/null | grep -c 'configuration fell back to' > /tmp/partial-recovery-fellback-before || true
        printf 'gnoblin.configure {keybindings = {wm = {panel_run_dialog = {"<Super>r"}}}, cursor = {size = 55}}\n' > "$F"
        printf 'gnoblin.configure {cursor = {size = 77}\n' > "$B"
        ;;
    read)
        echo "compositors=$(pgrep -x gnoblin | wc -l)"
        echo "status=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        echo "cursor=$("$G" config show 2>&1 | grep -o '"size":[0-9]*' | head -1)"
        echo "marker=$(head -1 "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null)"
        echo "marker_text=$(sed -n '2,$p' "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null | tr '\n' ' ')"
        echo "log=$(sudo journalctl -b --no-pager 2>/dev/null | grep 'gnoblin: ignored keybindings' | tail -1 | sed 's/.*gnoblin: //')"
        echo "file_log=$(sudo journalctl -b --no-pager 2>/dev/null | grep 'gnoblin: ignored file' | tail -1 | sed 's/.*gnoblin: //')"
        before=$(cat /tmp/partial-recovery-fellback-before 2>/dev/null || echo 0)
        after=$(sudo journalctl -b --no-pager 2>/dev/null | grep -c 'configuration fell back to' || true)
        echo "fellback=$((after - before))"
        ;;
    clean)
        rm -f "$F" "$B"
        ;;
    *)
        echo "usage: $0 install | read | clean" >&2
        exit 2
        ;;
esac
