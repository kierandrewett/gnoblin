#!/usr/bin/env bash
# Guest: helper for run-config-shortcut.sh. Usage: guest-config-shortcut.sh install BINDING | disable | hit | clean
#
# The shortcut runs `touch /tmp/cfg-shortcut-hit`. "hit" prints "hit" or "none" and removes the marker file.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-config-shortcut.lua"
MARK=/tmp/cfg-shortcut-hit

case "${1:-}" in
    install)
        rm -f "$MARK"
        printf 'gnoblin.configure {shortcuts = {cfg_test = {binding = "%s", command = {"touch", "%s"}}}}\n' "$2" "$MARK" > "$F"
        "$G" config reload | tail -1
        sleep 2
        ;;
    disable)
        rm -f "$MARK"
        printf 'gnoblin.configure {shortcuts = {cfg_test = {enable = false}}}\n' > "$F"
        "$G" config reload | tail -1
        sleep 2
        ;;
    hit)
        sleep 1
        if [ -e "$MARK" ]; then echo hit; else echo none; fi
        rm -f "$MARK"
        ;;
    clean)
        rm -f "$MARK" "$F"
        "$G" config reload | tail -1
        ;;
    *)
        echo "usage: $0 install BINDING | disable | hit | clean" >&2
        exit 2
        ;;
esac
