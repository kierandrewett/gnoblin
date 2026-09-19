#!/usr/bin/env bash
# Arch/CachyOS source-build prerequisites, shared by the guide and CI.
set -euo pipefail
privilege=()
if [ "$(id -u)" -ne 0 ]; then
    privilege=(sudo)
fi
"${privilege[@]}" pacman -Syu --needed "$@" \
    base-devel git just meson ninja python glib2-devel gobject-introspection \
    gnome-shell mutter gnome-session gnome-settings-daemon evolution-data-server \
    wayland-protocols egl-wayland libdisplay-info libei hyprcursor lua \
    sassc cmake intltool libxkbfile xorg-xwayland python-docutils
