#!/usr/bin/env bash
# Install the Fedora packages that ./build.sh needs to build Gnoblin, its Mutter and its portal backend.
#
# usage: install-fedora-build-deps.sh [--with-test-tools] [--allowerasing]
#
# Run as root in a Fedora container or on a Fedora host. This is the one list of build requirements for the Fedora
# builds: the package prefix build (scripts/build-prefix.sh), the full build with the test suite and the application
# end-to-end job all call it. Lua is not on the list: the prefix build links its own copy, and --with-test-tools adds the
# system one for the builds that run tests.
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
    'glib2-devel >= 2.86' gobject-introspection-devel 'gsettings-desktop-schemas-devel >= 49.1'
    gnome-desktop4-devel json-glib-devel lcms2-devel libcanberra-devel libdisplay-info-devel libdrm-devel
    libei-devel libeis-devel libgbm-devel libwacom-devel mesa-libEGL-devel mesa-libGL-devel mesa-libGLES-devel
    mesa-libgbm-devel pam-devel 'pipewire-devel >= 1.4.11' startup-notification-devel 'wayland-devel >= 1.25'
    'wayland-protocols-devel >= 1.48' xkeyboard-config-devel xorg-x11-proto-devel
    'pkgconfig(colord) >= 1.4.5' 'pkgconfig(gbm)' 'pkgconfig(gcr-4)' 'pkgconfig(gio-2.0)' 'pkgconfig(gio-unix-2.0)'
    'pkgconfig(glesv2)' 'pkgconfig(glib-2.0) >= 2.81.1' 'pkgconfig(glycin-2) >= 2.0.beta.2'
    'pkgconfig(gobject-introspection-1.0) >= 1.41.4' 'pkgconfig(graphene-gobject-1.0)' 'pkgconfig(gtk4) >= 4.14.0'
    'pkgconfig(gudev-1.0)' 'pkgconfig(ibus-1.0) >= 1.5.33' 'pkgconfig(json-glib-1.0)' 'pkgconfig(lcms2) >= 2.6'
    'pkgconfig(libcanberra)' 'pkgconfig(libdisplay-info) >= 0.2' 'pkgconfig(libdrm) >= 2.4.118'
    'pkgconfig(libei-1.0) >= 1.3.901' 'pkgconfig(libeis-1.0) >= 1.3.901' 'pkgconfig(libinput) >= 1.30.0'
    'pkgconfig(libpipewire-0.3) >= 1.4.11' 'pkgconfig(libstartup-notification-1.0)' 'pkgconfig(libsystemd)'
    'pkgconfig(libwacom)' 'pkgconfig(pixman-1) >= 0.42' 'pkgconfig(polkit-agent-1)' 'pkgconfig(polkit-gobject-1)'
    'pkgconfig(sm)' 'pkgconfig(wayland-protocols) >= 1.48' 'pkgconfig(wayland-server) >= 1.25'
    'pkgconfig(xkbcommon)' 'pkgconfig(xwayland)'
    # The portal backend
    'pkgconfig(fontconfig)' 'pkgconfig(gdk-pixbuf-2.0)' 'pkgconfig(gsettings-desktop-schemas)'
    'pkgconfig(gtk4) >= 4.20.0' 'pkgconfig(gtk4-unix-print)' 'pkgconfig(libadwaita-1) >= 1.7'
    'pkgconfig(xdg-desktop-portal) >= 1.20.0'
    # strip and patchelf for the prefix build
    binutils patchelf
)
if "$with_test_tools"; then
    # These builds link the system Lua, as a developer's build does. The prefix build links its own.
    packages+=('pkgconfig(lua) >= 5.4' adwaita-cursor-theme pkgconf-pkg-config util-linux-user xorg-x11-server-Xwayland)
fi

dnf -y install "${dnf_options[@]}" "${packages[@]}"
