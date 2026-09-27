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
    local family=$1 build_assume_yes=$2 build_dry_run=$3
    local -a privilege=() confirm=() frontend=() packages=() capabilities=()
    [ "$(id -u)" -eq 0 ] || privilege=(sudo)

    # RPM distributions resolve development packages by the interfaces they provide.
    local module
    for module in atk atk-bridge-2.0 cairo colord egl epoxy fribidi gbm gcr-4 \
        gdk-pixbuf-2.0 gio-2.0 girepository-2.0 gjs-1.0 gl glesv2 glycin-2 \
        gobject-introspection-1.0 graphene-gobject-1.0 gsettings-desktop-schemas \
        gtk4 gudev-1.0 json-glib-1.0 lcms2 \
        libcanberra libdisplay-info libdrm \
        libei-1.0 libeis-1.0 libinput libnm libpipewire-0.3 libpulse libsecret-1 \
        libstartup-notification-1.0 libseccomp libsystemd libudev libwacom pango pangocairo pixman-1 \
        polkit-agent-1 librsvg-2.0 sm systemd udev \
        wayland-client wayland-cursor wayland-egl wayland-server wayland-protocols \
        x11 x11-xcb xau xcb-res xcomposite xcursor xdamage xext xfixes xi \
        xinerama xkbcommon xkbregistry xrandr xwayland xdg-desktop-portal; do
        capabilities+=("pkgconfig($module)")
    done

    case "$family" in
        fedora)
            "$build_assume_yes" && confirm=(-y)
            packages=(git meson ninja-build python3 gcc gcc-c++ make cmake rpm-build adwaita-cursor-theme
                accountsservice-libs ibus-libs
                gettext gettext-devel pkgconf-pkg-config sassc desktop-file-utils readline-devel iso-codes
                python3-docutils python3-packaging glib2-devel libadwaita-devel expat-devel
                mesa-libEGL-devel
                pam-devel lua-devel cvt
                xkeyboard-config-devel xorg-x11-server-Xwayland)
            build_dependency_command "${privilege[@]}" dnf "${confirm[@]}" install \
                "${packages[@]}" "${capabilities[@]}"
            ;;
        arch)
            "$build_assume_yes" && confirm=(--noconfirm)
            packages=(base-devel git meson ninja python python-packaging accountsservice libibus
                glib2-devel gobject-introspection gjs gcr-4
                gtk4 libadwaita libcanberra libnm polkit startup-notification
                wayland-protocols egl-wayland libdisplay-info libei lua
                glycin libxkbcommon libxkbfile libxres
                sassc cmake gettext xdg-desktop-portal
                xorg-xwayland python-docutils adwaita-cursors)
            build_dependency_command "${privilege[@]}" pacman -S --needed \
                "${confirm[@]}" "${packages[@]}"
            ;;
        debian)
            if "$build_assume_yes"; then
                confirm=(-y)
                frontend=(env DEBIAN_FRONTEND=noninteractive)
            fi
            packages=(build-essential git meson ninja-build pkg-config cmake gettext
                python3 python3-docutils python3-packaging xcvt sassc desktop-file-utils
                adwaita-icon-theme
                gobject-introspection gir1.2-accountsservice-1.0 gir1.2-ibus-1.0
                libgirepository-2.0-dev libglib2.0-dev
                libgtk-4-dev libadwaita-1-dev libgjs-dev libglycin-2-dev
                liblua5.4-dev libatk-bridge2.0-dev libatk1.0-dev
                libcairo2-dev libcolord-dev libegl-dev libepoxy-dev libfribidi-dev
                libgbm-dev libgcr-4-dev libgdk-pixbuf-2.0-dev libgl-dev libgles-dev
                libgraphene-1.0-dev
                libgudev-1.0-dev libjson-glib-dev liblcms2-dev
                libcanberra-dev libdisplay-info-dev libdrm-dev
                libseccomp-dev libreadline-dev iso-codes libei-dev libeis-dev libinput-dev libnm-dev
                libpipewire-0.3-dev libpulse-dev libsecret-1-dev libstartup-notification0-dev libsystemd-dev
                libudev-dev libwacom-dev libpango1.0-dev libpixman-1-dev
                libpolkit-agent-1-dev librsvg2-dev libsm-dev libpam0g-dev
                libwayland-dev wayland-protocols libx11-dev libx11-xcb-dev libxau-dev
                libxcb-res0-dev libxcomposite-dev libxcursor-dev libxdamage-dev
                libxext-dev libxfixes-dev libxi-dev libxinerama-dev libxkbcommon-dev
                libxkbcommon-x11-dev libxkbregistry-dev libxrandr-dev xwayland
                xkb-data gsettings-desktop-schemas-dev xdg-desktop-portal-dev
                systemd-dev)
            build_dependency_command "${privilege[@]}" apt-get update
            build_dependency_command "${privilege[@]}" "${frontend[@]}" apt-get install --no-install-recommends "${confirm[@]}" "${packages[@]}"
            ;;
        opensuse)
            "$build_assume_yes" && confirm=(--non-interactive)
            packages=(git meson ninja python3 gcc gcc-c++ make cmake gettext-tools
                pkgconf-pkg-config sassc desktop-file-utils python3-docutils
                python3-packaging readline-devel iso-codes pam-devel lua54-devel
                adwaita-icon-theme)
            capabilities+=('pkgconfig(libadwaita-1)' 'pkgconfig(xkeyboard-config)')
            build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" refresh
            # Older minimal images gained busybox-gawk while bootstrapping Git,
            # but desktop-file-utils requires the full gawk implementation.
            # Current Tumbleweed no longer ships busybox-gawk, so only remove
            # it when it is actually installed.
            if rpm -q busybox-gawk >/dev/null 2>&1; then
                build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" remove busybox-gawk
            fi
            build_dependency_command "${privilege[@]}" zypper "${confirm[@]}" install \
                gawk "${packages[@]}" "${capabilities[@]}"
            ;;
        *)
            echo "No dependency setup for $family. See docs/install-source.md." >&2
            return 1
            ;;
    esac
}
