#!/usr/bin/env bash
# Install the system packages the Rust workspaces need on a hosted GitHub runner (Ubuntu 24.04) or in a container of
# one of the distributions CI tests on (Debian, Fedora, Arch Linux, openSUSE). This script is the one place the list
# lives: rust.yml, matrix.yml, wlcs.yml and scripts/parity-reference.sh run it and CONTRIBUTING.md names it, and none of
# them repeats the list.
#
#   scripts/ci-install-system-libraries.sh                what `cargo build` and `cargo test --workspace` of the root
#                                                         workspace and of extensions/gnoblin-xr need: the libraries
#                                                         they link and the programs and drivers the tests start or load
#   scripts/ci-install-system-libraries.sh GROUP ...      those, plus each GROUP:
#                                                           toolchain  the compiler, pkg-config, git, Node.js and Python
#                                                                      a bare distribution container lacks (a hosted
#                                                                      runner has them already)
#                                                           wlcs       what scripts/wlcs.sh builds wlcs with
#   scripts/ci-install-system-libraries.sh list GROUP ... print the packages for the groups that follow, install nothing
#
# The package manager is the first of apt-get, dnf, pacman and zypper that exists. DNF_OPTIONS adds options to dnf (a
# root file system without capabilities needs `--setopt=tsflags=nocaps`). It uses `sudo` when it is not root, so it
# runs on a hosted runner and in a container.
#
# The Ubuntu list was found by building both workspaces in a bare Ubuntu 24.04 root file system that held only
# build-essential, pkg-config, git, curl, python3 and nodejs, and installing what each pkg-config or linker error named,
# one package at a time. Each entry below is `debian|fedora|arch|suse`, the package of each family, with the error or
# test that needs it. `-` means the family needs nothing for that entry (another entry's package already provides it).
set -euo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

# Linked or probed by the build, and what the tests load or start from the system.
libraries=(
    "libudev-dev|systemd-devel|systemd-libs|systemd-devel"                                              # libudev-sys: pkg-config libudev
    "libseat-dev|libseat-devel|seatd|seatd-devel"                                                       # libseat-sys: pkg-config libseat
    "libpipewire-0.3-dev|pipewire-devel|pipewire|pipewire-devel"                                        # libspa-sys, pipewire-sys: pkg-config libpipewire-0.3
    "libclang-dev|clang-devel|clang|clang-devel"                                                        # bindgen, run by libspa-sys, needs libclang.so
    "libssl-dev|openssl-devel|openssl|libopenssl-devel"                                                 # sqlcipher, built from source by libsqlite3-sys: openssl/crypto.h
    "libxkbcommon-dev|libxkbcommon-devel|libxkbcommon|libxkbcommon-devel"                               # linker: -lxkbcommon
    "libpixman-1-dev|pixman-devel|pixman|libpixman-1-0-devel"                                           # linker: -lpixman-1
    "libinput-dev|libinput-devel|libinput|libinput-devel"                                               # linker: -linput
    "libgbm-dev|mesa-libgbm-devel|mesa|libgbm-devel"                                                    # linker: -lgbm
    "dbus-daemon|dbus-daemon|dbus|dbus-1-daemon"                                                        # the compat adapters' tests start a private bus and fail without it
    "pipewire|pipewire|pipewire|pipewire"                                                               # gnoblin-capture, gnoblin-pipewire-export and the cast tests start a private daemon
    "pipewire-bin|pipewire-utils|-|pipewire-tools"                                                      # pw-dump, which the cast and portal tests run (Arch's pipewire package has it)
    "xwayland|xorg-x11-server-Xwayland|xorg-xwayland|xwayland"                                          # three gnoblin-xwayland unit tests fail without an Xwayland on PATH; real X11 tests run with it
    "libegl1|mesa-libEGL|-|Mesa-libEGL1"                                                                # smithay opens libEGL.so.1 at run time and panics without it; pulls in Mesa's llvmpipe
    "libvulkan1|vulkan-loader|vulkan-icd-loader|libvulkan1"                                             # the Vulkan renderer's loader; its tests skip without a loader and a device
    "mesa-vulkan-drivers|mesa-vulkan-drivers|vulkan-swrast|libvulkan_lvp"                               # lavapipe, the software Vulkan device the renderer's 38 tests run on
    "vulkan-validationlayers|vulkan-validation-layers|vulkan-validation-layers|vulkan-validationlayers" # Khronos validation layer: the renderer's validation test skips without it
    "libwayland-dev|wayland-devel|wayland|wayland-devel"                                                # wayland-sys probes wayland-client and wayland-server with pkg-config unless its dlopen feature is on
    "libdrm-dev|libdrm-devel|libdrm|libdrm-devel"                                                       # drm-sys and smithay's DRM backend: pkg-config libdrm
    "libpython3-dev|python3-devel|python|python3-devel"                                                 # gnoblin-runtime-python: pyo3 finds python3 and links -lpython3.N
)

# What a bare distribution container lacks and a hosted runner has: the C toolchain the -sys crates and cc builds use,
# pkg-config, git and curl for the checkout and rustup, and the Node.js and Python the JavaScript and script checks run.
toolchain=(
    "build-essential|gcc|base-devel|gcc"
    "-|gcc-c++|-|gcc-c++"
    "-|make|-|make"
    "pkg-config|pkgconf-pkg-config|pkgconf|pkgconf-pkg-config"
    "git|git|git|git"
    "curl|curl|curl|curl"
    "ca-certificates|ca-certificates|ca-certificates|ca-certificates"
    "xz-utils|xz|xz|xz"
    "just|just|just|just"
    "procps|procps-ng|procps-ng|procps"
    "nodejs|nodejs|nodejs|nodejs-default"
    "npm|nodejs-npm|npm|npm-default"
    "python3|python3|python|python3"
)

# scripts/wlcs.sh builds Canonical's wlcs with cmake against the Wayland development files.
wlcs=(
    "cmake|cmake|cmake|cmake"
    "g++|gcc-c++|-|gcc-c++"
    "libgtest-dev|gtest-devel|gtest|gtest"
    "libgmock-dev|gmock-devel|-|gmock"
    "libboost-dev|boost-devel|boost|boost-devel"
)

mode=install
if [ "${1:-}" = list ]; then
    mode=list
    shift
fi

entries=("${libraries[@]}")
for group in "$@"; do
    case "$group" in
        toolchain) entries+=("${toolchain[@]}") ;;
        wlcs) entries+=("${wlcs[@]}") ;;
        *)
            echo "ci-install-system-libraries: unknown group '$group' (toolchain, wlcs)" >&2
            exit 2
            ;;
    esac
done

if command -v apt-get >/dev/null; then
    manager=apt column=1
elif command -v dnf >/dev/null; then
    manager=dnf column=2
elif command -v pacman >/dev/null; then
    manager=pacman column=3
elif command -v zypper >/dev/null; then
    manager=zypper column=4
else
    echo "ci-install-system-libraries: none of apt-get, dnf, pacman or zypper is available" >&2
    exit 1
fi

packages=()
for entry in "${entries[@]}"; do
    name=$(cut -d'|' -f"$column" <<<"$entry")
    [ "$name" = - ] || packages+=("$name")
done

if [ "$mode" = list ]; then
    printf '%s\n' "${packages[@]}"
    exit 0
fi

as_root=()
if [ "$(id -u)" -ne 0 ]; then
    as_root=(sudo)
fi

case "$manager" in
    apt)
        export DEBIAN_FRONTEND=noninteractive
        "${as_root[@]}" apt-get update
        "${as_root[@]}" apt-get install -y --no-install-recommends "${packages[@]}"
        ;;
    dnf)
        # shellcheck disable=SC2086 # DNF_OPTIONS is a list of options
        "${as_root[@]}" dnf -y ${DNF_OPTIONS:-} install "${packages[@]}"
        ;;
    pacman)
        # Arch supports no partial upgrade, so the package database and the installed system move together.
        "${as_root[@]}" pacman -Syu --noconfirm --needed "${packages[@]}"
        ;;
    zypper)
        "${as_root[@]}" zypper --non-interactive --gpg-auto-import-keys refresh
        "${as_root[@]}" zypper --non-interactive install --no-recommends "${packages[@]}"
        ;;
esac

# Fail here, with the name of what is missing, rather than in the middle of a build.
for module in libudev libseat libpipewire-0.3 libinput xkbcommon gbm pixman-1 openssl wayland-server libdrm; do
    pkg-config --exists "$module" || {
        echo "ci-install-system-libraries: pkg-config cannot find $module after the install" >&2
        exit 1
    }
done
for program in dbus-daemon pipewire pw-dump Xwayland; do
    command -v "$program" >/dev/null || {
        echo "ci-install-system-libraries: $program is not on PATH after the install" >&2
        exit 1
    }
done

# Xwayland refuses to start unless /tmp/.X11-unix exists with the sticky bit set. systemd creates it at boot on a
# hosted runner; a container does not have it.
"${as_root[@]}" install -d -m 1777 /tmp/.X11-unix
