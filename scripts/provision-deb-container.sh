#!/usr/bin/env bash
# Provision only disposable package-build containers, never an installed desktop.
set -euo pipefail
export PATH="$PATH"
if [ "$(id -u)" -ne 0 ] || { [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; }; then
    echo 'This script only runs as root inside a disposable Docker/Podman container.' >&2
    exit 2
fi
cd -- "$(dirname -- "$(realpath -- "$0")")/.."
source /etc/os-release
bundle=false
case "$ID:$VERSION_ID" in
    ubuntu:26.04) rust_packages=(rustc cargo libmozjs-140-dev) ;;
    ubuntu:24.04)
        bundle=true
        rust_packages=(rustc-1.85 cargo-1.85 g++-14)
        ;;
    debian:13)
        bundle=true
        rust_packages=(rustc cargo python3-legacy-cgi g++-14)
        ;;
    *)
        echo 'Supported build containers: Debian 13, Ubuntu 24.04 and Ubuntu 26.04.' >&2
        exit 2
        ;;
esac
source scripts/build-deps.sh
install_build_dependencies debian true false "$bundle"
apt-get install -y --no-install-recommends "${rust_packages[@]}" \
    python3-venv python3-jinja2 clang llvm cbindgen libreadline-dev zip zlib1g-dev valac \
    libheif-dev libjxl-dev libfontconfig-dev libevdev-dev hwdata libzip-dev libtomlplusplus-dev \
    bubblewrap dpkg-dev fakeroot patchelf curl ca-certificates foot \
    dbus-x11 xauth python3-gi-cairo gir1.2-accountsservice-1.0 gir1.2-upowerglib-1.0
python3 -m venv --system-site-packages /opt/gnoblin-build-tools
/opt/gnoblin-build-tools/bin/pip install meson==1.10.1
if [ "$ID:$VERSION_ID" = ubuntu:24.04 ]; then
    ln -sf /usr/bin/rustc-1.85 /opt/gnoblin-build-tools/bin/rustc
    ln -sf /usr/bin/cargo-1.85 /opt/gnoblin-build-tools/bin/cargo
    PATH="/opt/gnoblin-build-tools/bin:$PATH" cargo install --locked cbindgen --version 0.28.0 --root /opt/gnoblin-build-tools
fi
id builder >/dev/null 2>&1 || useradd -m builder
install -d -o builder -g builder /usr/lib/gnoblin
printf '%s\n' 'Ready. As builder, with /opt/gnoblin-build-tools/bin on PATH, run scripts/build-deb.sh.'
