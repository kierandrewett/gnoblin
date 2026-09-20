#!/usr/bin/env bash
# Disposable CI image provisioning only. Never called by build.sh.

build_distro_family() {
    local candidate
    for candidate in "$1" ${2:-}; do
        case "$candidate" in
            fedora)
                echo fedora
                return
                ;;
            arch)
                echo arch
                return
                ;;
            debian | ubuntu)
                echo debian
                return
                ;;
            opensuse* | suse)
                echo opensuse
                return
                ;;
        esac
    done
    return 1
}

build_dependency_command() {
    if "$build_dry_run"; then
        printf '  '
        printf '%q ' "$@"
        printf '\n'
    else
        "$@"
    fi
}

install_build_dependencies() {
    local family=$1 build_assume_yes=$2 build_dry_run=$3 bundle_debian=${4:-false}
    local -a privilege=() confirm=() frontend=() packages=() capabilities=()
    [ "$(id -u)" -eq 0 ] || privilege=(sudo)

    # RPM distributions resolve development packages by the interfaces they provide.
    local module
    for module in atk atk-bridge-2.0 cairo colord egl epoxy fribidi gbm gcr-4 \
        gdk-pixbuf-2.0 gio-2.0 girepository-2.0 gjs-1.0 gl glesv2 glycin-2 \
        gnome-autoar-0 gnome-desktop-4 gnome-settings-daemon \
        gobject-introspection-1.0 graphene-gobject-1.0 gsettings-desktop-schemas \
        gstreamer-base-1.0 gtk4 gudev-1.0 hyprcursor json-glib-1.0 lcms2 \
        libcanberra libdisplay-info libdrm libecal-2.0 libedataserver-1.2 \
        libei-1.0 libeis-1.0 libinput libnm libpipewire-0.3 libpulse libsecret-1 \
        libstartup-notification-1.0 libseccomp libsystemd libudev libwacom libxml-2.0 pango pangocairo pixman-1 \
        polkit-agent-1 librsvg-2.0 sm systemd sysprof-capture-4 udev \
        wayland-client wayland-cursor wayland-egl wayland-server wayland-protocols \
        x11 x11-xcb xau xcb-res xcomposite xcursor xdamage xext xfixes xi \
        xinerama xkbcommon xkbregistry xrandr xwayland; do
        capabilities+=("pkgconfig($module)")
    done

    case "$family" in
        fedora)
            "$build_assume_yes" && confirm=(-y)
            packages=(git just meson ninja-build python3 gcc gcc-c++ make cmake
                gettext gettext-devel pkgconf-pkg-config sassc desktop-file-utils readline-devel iso-codes
                python3-docutils python3-packaging glib2-devel libadwaita-devel expat-devel
                mesa-libEGL-devel
                pam-devel lua-devel cvt gnome-shell gnome-session gnome-settings-daemon
                xkeyboard-config-devel xorg-x11-server-Xwayland)
            build_dependency_command "${privilege[@]}" dnf "${confirm[@]}" install \
                "${packages[@]}" "${capabilities[@]}"
            ;;
        arch)
            "$build_assume_yes" && confirm=(--noconfirm)
            packages=(base-devel git just meson ninja python python-packaging
                glib2-devel gobject-introspection gjs gtk4 libadwaita
                gnome-shell mutter gnome-session gnome-settings-daemon
                wayland-protocols egl-wayland libdisplay-info libei hyprcursor lua
                glycin libxkbcommon libxkbfile libxres sysprof evolution-data-server
                sassc cmake gettext xorg-xwayland python-docutils)
            build_dependency_command "${privilege[@]}" pacman -S --needed \
                "${confirm[@]}" "${packages[@]}"
            ;;
        debian)
            if "$build_assume_yes"; then
                confirm=(-y)
                frontend=(env DEBIAN_FRONTEND=noninteractive)
            fi
            packages=(build-essential git just meson ninja-build pkg-config cmake gettext
                python3 python3-docutils python3-packaging python3-argcomplete xcvt sassc desktop-file-utils
                gobject-introspection libgirepository-2.0-dev libglib2.0-dev
                libgtk-4-dev libadwaita-1-dev libgjs-dev libglycin-2-dev
                libhyprcursor-dev liblua5.4-dev libatk-bridge2.0-dev libatk1.0-dev
                libcairo2-dev libcolord-dev libegl-dev libepoxy-dev libfribidi-dev
                libgbm-dev libgcr-4-dev libgdk-pixbuf-2.0-dev libgl-dev libgles-dev
                libgnome-autoar-0-dev libgnome-desktop-4-dev libgraphene-1.0-dev
                libgstreamer1.0-dev libgudev-1.0-dev libjson-glib-dev liblcms2-dev
                libcanberra-dev libdisplay-info-dev libdrm-dev libecal2.0-dev
                libedataserver1.2-dev libseccomp-dev libreadline-dev iso-codes libei-dev libeis-dev libinput-dev libnm-dev
                libpipewire-0.3-dev libpulse-dev libsecret-1-dev libstartup-notification0-dev libsystemd-dev
                libudev-dev libwacom-dev libxml2-dev libpango1.0-dev libpixman-1-dev
                libpolkit-agent-1-dev librsvg2-dev libsm-dev libsysprof-capture-4-dev libpam0g-dev
                libwayland-dev wayland-protocols libx11-dev libx11-xcb-dev libxau-dev
                libxcb-res0-dev libxcomposite-dev libxcursor-dev libxdamage-dev
                libxext-dev libxfixes-dev libxi-dev libxinerama-dev libxkbcommon-dev
                libxkbcommon-x11-dev libxkbregistry-dev libxrandr-dev xwayland
                xkb-data gsettings-desktop-schemas-dev gnome-settings-daemon-dev
                gnome-shell gnome-session-bin gnome-session-common gnome-settings-daemon systemd-dev)
            if "$bundle_debian"; then
                local -a base_packages=()
                local package
                for package in "${packages[@]}"; do
                    case "$package" in
                        libglycin-2-dev | libhyprcursor-dev | libgjs-dev) ;;
                        *) base_packages+=("$package") ;;
                    esac
                done
                packages=("${base_packages[@]}")
            fi
            build_dependency_command "${privilege[@]}" apt-get update
            build_dependency_command "${privilege[@]}" "${frontend[@]}" apt-get install --no-install-recommends "${confirm[@]}" "${packages[@]}"
            ;;
        opensuse)
            "$build_assume_yes" && confirm=(--non-interactive)
            packages=(git just meson ninja python3 gcc gcc-c++ make cmake gettext-tools
                pkgconf-pkg-config sassc desktop-file-utils python3-docutils
                python3-packaging readline-devel iso-codes pam-devel lua54-devel gnome-shell gnome-session gnome-settings-daemon)
            capabilities+=('pkgconfig(libadwaita-1)' 'pkgconfig(xkeyboard-config)')
            build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" refresh
            # The minimal CI image gains busybox-gawk while bootstrapping Git,
            # but desktop-file-utils requires the full gawk implementation.
            build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" remove busybox-gawk
            build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" install \
                gawk "${packages[@]}" "${capabilities[@]}"
            ;;
        *)
            echo "No dependency setup for $family. See docs/install-source.md." >&2
            return 1
            ;;
    esac
}
