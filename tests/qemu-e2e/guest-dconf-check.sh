#!/usr/bin/env bash
# Guest: after Gnoblin sessions, did anything write GNOME keybinding settings into the user dconf database?
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
echo "== user dconf database modified: $(stat -c %y "$HOME/.config/dconf/user" 2>&1 | cut -c1-19)"
for path in /org/gnome/desktop/wm/keybindings/ /org/gnome/mutter/keybindings/ /org/gnome/shell/keybindings/ /org/gnome/mutter/; do
    echo "== dconf dump $path"
    dconf dump "$path" 2>&1 | head -20
done
echo "== effective values"
for pair in "org.gnome.desktop.wm.keybindings switch-applications" "org.gnome.desktop.wm.keybindings switch-windows" "org.gnome.mutter overlay-key" "org.gnome.mutter edge-tiling" "org.gnome.mutter dynamic-workspaces"; do
    set -- $pair
    printf '%s %s = %s\n' "$1" "$2" "$(gsettings get "$1" "$2")"
done
