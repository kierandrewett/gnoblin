#!/usr/bin/env bash
# Install the Alpine packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend.
#
# usage: install-alpine-build-deps.sh [--with-test-tools]
#
# Run as root in an Alpine container: edge, or a release branch such as 3.22. It is the Alpine counterpart of
# scripts/install-fedora-build-deps.sh, for the musl prefix that the apk packages come from (scripts/build-prefix.sh).
# Each branch that has a job builds its own prefix with this script. Lua is not on the list: the prefix build links its
# own copy, and --with-test-tools adds the system one for ./build.sh, which links it.
#
# The list names no version: the version floors come from the pinned Meson files, which scripts/check-build-deps.py
# reads before ./build.sh compiles. A package on the optional list is installed when the index of the branch has it, and
# left out with a message when it does not. Glycin is optional: the portal backend builds without it
# (patches/xdg-desktop-portal-gnome/zz-older-systems), and Alpine 3.23 and older have no libglycin-dev.
set -euo pipefail

with_test_tools=false
for argument in "$@"; do
    case "$argument" in
        --with-test-tools) with_test_tools=true ;;
        *)
            echo "usage: $0 [--with-test-tools]" >&2
            exit 2
            ;;
    esac
done

packages=(
    # Tools
    bash cmake samurai meson git patch tar xz gzip curl python3 py3-docutils py3-packaging py3-gobject3 gcc g++ musl-dev
    make pkgconf binutils coreutils findutils gettext-dev desktop-file-utils libxcvt
    # Mutter
    glib-dev gobject-introspection-dev gsettings-desktop-schemas-dev gnome-desktop-dev json-glib-dev lcms2-dev
    libcanberra-dev libdisplay-info-dev libdrm-dev libei-dev mesa-dev mesa-gbm linux-pam-dev pipewire-dev
    startup-notification-dev wayland-dev wayland-protocols libxkbcommon-dev xkeyboard-config-dev xorgproto libinput-dev
    libwacom-dev gtk4.0-dev gcr4-dev polkit-dev elogind-dev eudev-dev libgudev-dev colord-dev ibus-dev
    graphene-dev harfbuzz-dev pango-dev fribidi-dev at-spi2-core-dev cairo-dev pixman-dev libsm-dev libice-dev
    libx11-dev libxi-dev libxfixes-dev libxcomposite-dev libxcursor-dev libxrandr-dev libxinerama-dev libxext-dev
    libxau-dev libxdamage-dev libxkbfile-dev libxres-dev libxcb-dev xwayland fontconfig-dev gdk-pixbuf-dev
    # The portal backend
    libadwaita-dev xdg-desktop-portal-dev libsecret-dev sassc gcr-dev
)
if "$with_test_tools"; then
    packages+=(lua5.4-dev)
fi
# Installed only where the branch has them.
optional=(libglycin-dev)

apk update
for package in "${optional[@]}"; do
    if [ -n "$(apk search -x "$package")" ]; then
        packages+=("$package")
    else
        echo "[alpine-deps] $package is not in this branch; building without it"
    fi
done
apk add --no-cache "${packages[@]}"
