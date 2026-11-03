#!/usr/bin/env bash
# Install the Fedora packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend.
#
# usage: install-fedora-build-deps.sh [--with-test-tools] [--allowerasing]
#
# Run as root in a Fedora container or on a Fedora host, Fedora 42 or newer. This is the one list of build requirements
# for the Fedora builds: the package prefix build (scripts/build-prefix.sh), the full build with the test suite and the
# application end-to-end job all call it. Lua is not on the list: the prefix build links its own copy, and
# --with-test-tools adds the system one for the builds that run tests.
#
# The list names no version. dnf installs the newest package of the release, and the version floors come from the pinned
# Meson files: scripts/check-build-deps.py reads them and ./build.sh runs it before it compiles, so a release that is too
# old stops there with the library and the version it lacks. Glycin is optional: the portal backend builds without it
# (patches/xdg-desktop-portal-gnome/zz-older-systems), so a release that has no Glycin package installs the rest.
#
# --with-test-tools also installs what the CTest suites and the end-to-end jobs run besides the build. --allowerasing lets
# dnf replace an installed package that conflicts with one the build needs.
set -euo pipefail

with_test_tools=false
dnf_options=(--setopt=install_weak_deps=False)
for argument in "$@"; do
    case "$argument" in
        --with-test-tools) with_test_tools=true ;;
        --allowerasing) dnf_options+=(--allowerasing) ;;
        *)
            echo "usage: $0 [--with-test-tools] [--allowerasing]" >&2
            exit 2
            ;;
    esac
done

packages=(
    # Tools
    cmake ninja-build meson gcc gcc-c++ make git-core patch tar gzip xz python3 python3-docutils python3-packaging
    redhat-rpm-config cvt desktop-file-utils gettext gettext-devel
    # Mutter
    glib2-devel gobject-introspection-devel gsettings-desktop-schemas-devel
    gnome-desktop4-devel json-glib-devel lcms2-devel libcanberra-devel libdisplay-info-devel libdrm-devel
    libei-devel libeis-devel libgbm-devel libwacom-devel mesa-libEGL-devel mesa-libGL-devel mesa-libGLES-devel
    mesa-libgbm-devel pam-devel pipewire-devel startup-notification-devel wayland-devel
    wayland-protocols-devel xkeyboard-config-devel xorg-x11-proto-devel
    'pkgconfig(colord)' 'pkgconfig(gbm)' 'pkgconfig(gcr-4)' 'pkgconfig(gio-2.0)' 'pkgconfig(gio-unix-2.0)'
    'pkgconfig(glesv2)' 'pkgconfig(glib-2.0)' 'pkgconfig(gobject-introspection-1.0)' 'pkgconfig(graphene-gobject-1.0)'
    'pkgconfig(gudev-1.0)' 'pkgconfig(ibus-1.0)' 'pkgconfig(json-glib-1.0)' 'pkgconfig(lcms2)'
    'pkgconfig(libcanberra)' 'pkgconfig(libdisplay-info)' 'pkgconfig(libdrm)'
    'pkgconfig(libei-1.0)' 'pkgconfig(libeis-1.0)' 'pkgconfig(libinput)'
    'pkgconfig(libpipewire-0.3)' 'pkgconfig(libstartup-notification-1.0)' 'pkgconfig(libsystemd)'
    'pkgconfig(libwacom)' 'pkgconfig(pixman-1)' 'pkgconfig(polkit-agent-1)' 'pkgconfig(polkit-gobject-1)'
    'pkgconfig(sm)' 'pkgconfig(wayland-protocols)' 'pkgconfig(wayland-server)'
    'pkgconfig(xkbcommon)' 'pkgconfig(xwayland)'
    # Mutter's X11 frames client and the portal backend
    'pkgconfig(gtk4)' 'pkgconfig(fontconfig)' 'pkgconfig(gdk-pixbuf-2.0)' 'pkgconfig(gsettings-desktop-schemas)'
    'pkgconfig(gtk4-unix-print)' 'pkgconfig(libadwaita-1)' 'pkgconfig(xdg-desktop-portal)'
    # strip and patchelf for the prefix build
    binutils patchelf
)
if "$with_test_tools"; then
    # These builds link the system Lua, as a developer's build does. The prefix build links its own.
    packages+=('pkgconfig(lua)' adwaita-cursor-theme pkgconf-pkg-config util-linux-user xorg-x11-server-Xwayland)
fi
# Optional: the portal backend decodes wallpaper previews in the Glycin sandbox when the release has Glycin.
optional=('pkgconfig(glycin-2)')

dnf -y install "${dnf_options[@]}" "${packages[@]}"
dnf -y install "${dnf_options[@]}" --skip-unavailable "${optional[@]}"
