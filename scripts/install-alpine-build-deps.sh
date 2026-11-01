#!/usr/bin/env bash
# Install the Alpine packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend on Alpine edge.
#
# usage: install-alpine-build-deps.sh
#
# Run as root in an alpine:edge container. It is the Alpine counterpart of scripts/install-fedora-build-deps.sh, for the
# musl prefix that the apk packages come from (scripts/build-prefix.sh). Lua is not on the list: the prefix build links
# its own copy.
set -euo pipefail

apk add --no-cache \
    bash cmake samurai meson git patch tar xz gzip curl python3 py3-docutils py3-packaging py3-gobject3 gcc g++ musl-dev \
    make pkgconf binutils coreutils findutils gettext-dev desktop-file-utils libxcvt \
    glib-dev gobject-introspection-dev gsettings-desktop-schemas-dev gnome-desktop-dev json-glib-dev lcms2-dev \
    libcanberra-dev libdisplay-info-dev libdrm-dev libei-dev mesa-dev mesa-gbm linux-pam-dev pipewire-dev \
    startup-notification-dev wayland-dev wayland-protocols libxkbcommon-dev xkeyboard-config-dev xorgproto libinput-dev \
    libwacom-dev gtk4.0-dev libglycin-dev gcr4-dev polkit-dev elogind-dev eudev-dev libgudev-dev colord-dev ibus-dev \
    graphene-dev harfbuzz-dev pango-dev fribidi-dev at-spi2-core-dev cairo-dev pixman-dev libsm-dev libice-dev \
    libx11-dev libxi-dev libxfixes-dev libxcomposite-dev libxcursor-dev libxrandr-dev libxinerama-dev libxext-dev \
    libxau-dev libxdamage-dev libxkbfile-dev libxres-dev libxcb-dev xwayland fontconfig-dev gdk-pixbuf-dev \
    libadwaita-dev xdg-desktop-portal-dev libsecret-dev sassc gcr-dev
