#!/usr/bin/env bash
# Install the system packages the Rust workspaces need on a hosted GitHub runner (Ubuntu 24.04) or in a Fedora
# container. This script is the one place the list lives: rust.yml, wlcs.yml and scripts/parity-reference.sh run it and
# CONTRIBUTING.md names it, and none of them repeats the list.
#
#   scripts/ci-install-system-libraries.sh            what `cargo build` and `cargo test --workspace` of the root
#                                                     workspace and of extensions/gnoblin-xr need: the libraries they
#                                                     link and the programs and drivers the tests start or load
#   scripts/ci-install-system-libraries.sh wlcs       those, plus what scripts/wlcs.sh builds wlcs with
#   scripts/ci-install-system-libraries.sh list ...   print the packages for the groups that follow, install nothing
#
# The package manager is apt-get when there is one and dnf otherwise; DNF_OPTIONS adds options to dnf (a root file
# system without capabilities needs `--setopt=tsflags=nocaps`). It uses `sudo` when it is not root, so it runs on a
# hosted runner and in a container.
#
# The Ubuntu list was found by building both workspaces in a bare Ubuntu 24.04 root file system that held only
# build-essential, pkg-config, git, curl, python3 and nodejs, and installing what each pkg-config or linker error named,
# one package at a time. Each entry below is `debian-name|fedora-name`, with the error or test that needs it.
set -euo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

# Linked or probed by the build, and what the tests load or start from the system.
libraries=(
    "libudev-dev|systemd-devel"               # libudev-sys: pkg-config libudev (libinput-dev pulls it in on Debian)
    "libseat-dev|libseat-devel"               # libseat-sys: pkg-config libseat
    "libpipewire-0.3-dev|pipewire-devel"      # libspa-sys, pipewire-sys: pkg-config libpipewire-0.3
    "libclang-dev|clang-devel"                # bindgen, run by libspa-sys, needs libclang.so
    "libssl-dev|openssl-devel"                # sqlcipher, built from source by libsqlite3-sys: openssl/crypto.h
    "libxkbcommon-dev|libxkbcommon-devel"     # linker: -lxkbcommon
    "libpixman-1-dev|pixman-devel"            # linker: -lpixman-1
    "libinput-dev|libinput-devel"             # linker: -linput
    "libgbm-dev|mesa-libgbm-devel"            # linker: -lgbm
    "dbus-daemon|dbus-daemon"                 # the compat adapters' tests start a private bus and fail without it
    "pipewire|pipewire"                       # gnoblin-capture, gnoblin-pipewire-export and the cast tests start a private daemon
    "pipewire-bin|pipewire-utils"             # pw-dump, which the cast and portal tests run (the pipewire package pulls it in on Debian)
    "xwayland|xorg-x11-server-Xwayland"       # three gnoblin-xwayland unit tests fail without an Xwayland on PATH; real X11 tests run with it
    "libegl1|mesa-libEGL"                     # smithay opens libEGL.so.1 at run time and panics without it; pulls in Mesa's llvmpipe
    "libvulkan1|vulkan-loader"                # the Vulkan renderer's loader; its tests skip without a loader and a device
    "mesa-vulkan-drivers|mesa-vulkan-drivers" # lavapipe, the software Vulkan device the renderer's 38 tests run on
    "libwayland-dev|wayland-devel"            # wayland-sys probes wayland-client and wayland-server with pkg-config unless its dlopen feature is on
    "libdrm-dev|libdrm-devel"                 # drm-sys and smithay's DRM backend: pkg-config libdrm
)

# scripts/wlcs.sh builds Canonical's wlcs with cmake against the Wayland development files.
wlcs=(
    "cmake|cmake"
    "g++|gcc-c++"
    "libgtest-dev|gtest-devel"
    "libgmock-dev|gmock-devel"
    "libboost-dev|boost-devel"
)

mode=install
if [ "${1:-}" = list ]; then
    mode=list
    shift
fi

entries=("${libraries[@]}")
for group in "$@"; do
    case "$group" in
        wlcs) entries+=("${wlcs[@]}") ;;
        *)
            echo "ci-install-system-libraries: unknown group '$group' (wlcs)" >&2
            exit 2
            ;;
    esac
done

if command -v apt-get >/dev/null; then
    manager=apt
    packages=("${entries[@]%%|*}")
elif command -v dnf >/dev/null; then
    manager=dnf
    packages=("${entries[@]#*|}")
else
    echo "ci-install-system-libraries: neither apt-get nor dnf is available" >&2
    exit 1
fi

if [ "$mode" = list ]; then
    printf '%s\n' "${packages[@]}"
    exit 0
fi

as_root=()
if [ "$(id -u)" -ne 0 ]; then
    as_root=(sudo)
fi

if [ "$manager" = apt ]; then
    export DEBIAN_FRONTEND=noninteractive
    "${as_root[@]}" apt-get update
    "${as_root[@]}" apt-get install -y --no-install-recommends "${packages[@]}"
else
    # shellcheck disable=SC2086 # DNF_OPTIONS is a list of options
    "${as_root[@]}" dnf -y ${DNF_OPTIONS:-} install "${packages[@]}"
fi

# Fail here, with the name of what is missing, rather than in the middle of a build.
for module in libudev libseat libpipewire-0.3 libinput xkbcommon gbm pixman-1 openssl; do
    pkg-config --exists "$module" || {
        echo "ci-install-system-libraries: pkg-config cannot find $module after the install" >&2
        exit 1
    }
done
command -v dbus-daemon >/dev/null || {
    echo "ci-install-system-libraries: dbus-daemon is not on PATH after the install" >&2
    exit 1
}

# Xwayland refuses to start unless /tmp/.X11-unix exists with the sticky bit set. systemd creates it at boot on a
# hosted runner; a container does not have it.
"${as_root[@]}" install -d -m 1777 /tmp/.X11-unix
