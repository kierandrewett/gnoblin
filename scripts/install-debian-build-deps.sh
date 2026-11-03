#!/usr/bin/env bash
# Install the Debian or Ubuntu packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend.
#
# usage: install-debian-build-deps.sh [--with-test-tools]
#
# Run as root in a debian:13, ubuntu:24.04 or ubuntu:26.04 container, or a newer release. It is the Debian counterpart
# of scripts/install-fedora-build-deps.sh. apt cannot ask for a minimum version, so the version floors are checked by
# scripts/check-build-deps.py, which ./build.sh runs before it compiles. Lua is not on the list: the prefix build links
# its own copy, and --with-test-tools adds the system one for ./build.sh, which links it.
#
# Mutter needs Meson 1.5 or newer. When the release has an older Meson (Ubuntu 24.04 has 1.3.2), the script installs
# Meson 1.5.2 from PyPI into /opt/gnoblin-meson and links it as /usr/local/bin/meson, which comes before /usr/bin in
# PATH. Meson is a build tool only: nothing that it installs goes into the packages.
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
    cmake ninja-build meson gcc g++ make git patch tar gzip xz-utils curl ca-certificates python3 python3-docutils
    python3-packaging pkg-config xcvt desktop-file-utils gettext
    # Mutter
    libglib2.0-dev libglib2.0-bin gobject-introspection libgirepository1.0-dev gsettings-desktop-schemas-dev
    libgnome-desktop-4-dev libgnome-bg-4-dev libjson-glib-dev liblcms2-dev libcanberra-dev libcolord-dev
    libdisplay-info-dev libdrm-dev
    libei-dev libeis-dev libgbm-dev libegl-dev libgl-dev libgles-dev libepoxy-dev libwacom-dev libpam0g-dev
    libstartup-notification0-dev libpipewire-0.3-dev libwayland-dev wayland-protocols libinput-dev libudev-dev
    libgudev-1.0-dev libsystemd-dev systemd-dev libgraphene-1.0-dev libgcr-4-dev libibus-1.0-dev libpixman-1-dev
    libpolkit-agent-1-dev libpolkit-gobject-1-dev libsm-dev libxkbcommon-dev libxkbcommon-x11-dev libxkbregistry-dev
    libxkbfile-dev xkb-data x11proto-dev libx11-dev libx11-xcb-dev libxau-dev libxcomposite-dev libxcursor-dev
    libxdamage-dev libxext-dev libxfixes-dev libxi-dev libxinerama-dev libxrandr-dev libxtst-dev libxcb1-dev
    libxcb-res0-dev libxcvt-dev xwayland libcairo2-dev libpango1.0-dev libharfbuzz-dev libfribidi-dev libatk1.0-dev
    libgtk-4-dev hwdata
    # The portal backend
    libfontconfig-dev libgdk-pixbuf-2.0-dev libadwaita-1-dev xdg-desktop-portal-dev libsecret-1-dev
    # strip, readelf and objdump for the prefix and package builds
    binutils patchelf
)
if "$with_test_tools"; then
    packages+=(liblua5.4-dev)
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends "${packages[@]}"

if ! python3 -c 'import sys; from packaging.version import Version; sys.exit(Version(sys.argv[1]) < Version("1.5.0"))' \
    "$(meson --version)"; then
    apt-get install -y --no-install-recommends python3-venv
    python3 -m venv /opt/gnoblin-meson
    /opt/gnoblin-meson/bin/pip install --no-cache-dir meson==1.5.2
    ln -sf /opt/gnoblin-meson/bin/meson /usr/local/bin/meson
fi
