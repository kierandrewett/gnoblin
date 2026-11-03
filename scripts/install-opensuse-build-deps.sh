#!/usr/bin/env bash
# Install the openSUSE packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend.
#
# usage: install-opensuse-build-deps.sh [--with-test-tools]
#
# Run as root in an openSUSE Leap 16.0 or newer, or a Tumbleweed container. It is the openSUSE counterpart of
# scripts/install-fedora-build-deps.sh, for the prefix that a Leap release builds its own rpm from
# (scripts/build-prefix.sh). Lua is not on the list: the prefix build links its own copy, and --with-test-tools adds the
# system one for ./build.sh, which links it.
#
# Most packages are named by the pkg-config module they provide, because openSUSE splits and names its development
# packages in its own way. The list names no version: the version floors come from the pinned Meson files, which
# scripts/check-build-deps.py reads before ./build.sh compiles. A package on the optional list is installed when the
# release has it, and left out with a message when it does not. Glycin is optional: the portal backend builds without it
# (patches/xdg-desktop-portal-gnome/zz-older-systems), and Leap 16.0 has no Glycin package.
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
    # Tools. Mutter runs the cvt program of libxcvt.
    cmake ninja meson gcc gcc-c++ make git-core patch tar gzip xz curl which python3 python3-docutils python3-packaging
    pkgconf-pkg-config desktop-file-utils gettext-tools gettext-runtime libxcvt xkeyboard-config
    # Mutter
    gobject-introspection-devel gsettings-desktop-schemas-devel
    'pkgconfig(colord)' 'pkgconfig(egl)' 'pkgconfig(gbm)' 'pkgconfig(gcr-4)' 'pkgconfig(gio-2.0)'
    'pkgconfig(gio-unix-2.0)' 'pkgconfig(gl)' 'pkgconfig(glesv2)' 'pkgconfig(glib-2.0)' 'pkgconfig(gnome-bg-4)'
    'pkgconfig(gnome-desktop-4)' 'pkgconfig(graphene-gobject-1.0)' 'pkgconfig(gudev-1.0)' 'pkgconfig(ibus-1.0)'
    'pkgconfig(json-glib-1.0)' 'pkgconfig(lcms2)' 'pkgconfig(libcanberra)' 'pkgconfig(libdisplay-info)'
    'pkgconfig(libdrm)' 'pkgconfig(libei-1.0)' 'pkgconfig(libeis-1.0)' 'pkgconfig(libinput)'
    'pkgconfig(libpipewire-0.3)' 'pkgconfig(libstartup-notification-1.0)' 'pkgconfig(libsystemd)'
    'pkgconfig(libwacom)' 'pkgconfig(pam)' 'pkgconfig(pixman-1)' 'pkgconfig(polkit-agent-1)'
    'pkgconfig(polkit-gobject-1)' 'pkgconfig(sm)' 'pkgconfig(udev)' 'pkgconfig(wayland-protocols)'
    'pkgconfig(wayland-server)' 'pkgconfig(xkbcommon)' 'pkgconfig(xkbfile)' 'pkgconfig(xwayland)'
    'pkgconfig(x11)' 'pkgconfig(xcomposite)' 'pkgconfig(xcursor)' 'pkgconfig(xdamage)' 'pkgconfig(xfixes)'
    'pkgconfig(xi)' 'pkgconfig(xinerama)' 'pkgconfig(xrandr)'
    # Mutter's X11 frames client and the portal backend
    'pkgconfig(atk)' 'pkgconfig(cairo)' 'pkgconfig(fontconfig)' 'pkgconfig(fribidi)' 'pkgconfig(gdk-pixbuf-2.0)'
    'pkgconfig(gsettings-desktop-schemas)' 'pkgconfig(gtk4)' 'pkgconfig(gtk4-unix-print)' 'pkgconfig(harfbuzz)'
    'pkgconfig(libadwaita-1)' 'pkgconfig(pango)' 'pkgconfig(xdg-desktop-portal)'
    # strip and patchelf for the prefix build
    binutils patchelf
)
if "$with_test_tools"; then
    packages+=('pkgconfig(lua5.4)')
fi
# Installed only where the release has them.
optional=('pkgconfig(glycin-2)')

zypper -n --gpg-auto-import-keys refresh
for package in "${optional[@]}"; do
    if zypper -n --quiet search --provides --match-exact "$package" >/dev/null 2>&1; then
        packages+=("$package")
    else
        echo "[opensuse-deps] $package is not in this release; building without it"
    fi
done
zypper -n install --no-recommends "${packages[@]}"
