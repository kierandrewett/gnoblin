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
legacy_private_gtk=false
private_deb_addons=false
case "$ID:$VERSION_ID" in
    ubuntu:26.04) rust_packages=(rustc cargo libmozjs-140-dev) ;;
    ubuntu:24.04)
        private_deb_addons=true
        rust_packages=(rustc-1.85 cargo-1.85 g++-14)
        ;;
    debian:13)
        private_deb_addons=true
        rust_packages=(rustc cargo python3-legacy-cgi g++-14)
        ;;
    debian:12 | ubuntu:22.04)
        echo "$(tr '[:lower:]' '[:upper:]' <<<"$ID") $VERSION_ID uses the private compatibility build path." >&2
        echo 'Run scripts/provision-deb-compat-container.sh, then scripts/build-deb-compat-runtime.sh.' >&2
        exit 2
        ;;
    *)
        echo 'Supported build containers: Debian 13, Ubuntu 24.04 and Ubuntu 26.04.' >&2
        echo 'Debian 11/12 and Ubuntu 22.04 use scripts/provision-deb-compat-container.sh.' >&2
        exit 2
        ;;
esac
source scripts/build-deps.sh
install_build_dependencies debian true false "$legacy_private_gtk" "$private_deb_addons"
apt-get install -y --no-install-recommends "${rust_packages[@]}" \
    python3-venv python3-jinja2 clang llvm cbindgen libreadline-dev zip zlib1g-dev valac \
    libheif-dev libjxl-dev libfontconfig-dev libevdev-dev hwdata libzip-dev libtomlplusplus-dev \
    bubblewrap dpkg-dev fakeroot patchelf curl ca-certificates foot \
    dbus-x11 xauth python3-gi-cairo gir1.2-accountsservice-1.0 gir1.2-upowerglib-1.0
if [ "$ID:$VERSION_ID" = ubuntu:26.04 ]; then
    # Mutter requires the generic udev pkg-config module. On Ubuntu 26.04
    # libudev-dev exports libudev.pc, while systemd-dev exports udev.pc.
    apt-get install -y --no-install-recommends hyprcursor-util systemd-dev
fi
python3 -m venv --system-site-packages /opt/gnoblin-build-tools
/opt/gnoblin-build-tools/bin/pip install meson==1.10.1
if [ "$ID:$VERSION_ID" = ubuntu:24.04 ]; then
    ln -sf /usr/bin/rustc-1.85 /opt/gnoblin-build-tools/bin/rustc
    ln -sf /usr/bin/cargo-1.85 /opt/gnoblin-build-tools/bin/cargo
    export PATH="/opt/gnoblin-build-tools/bin:$PATH"
    cargo install --locked cbindgen --version 0.28.0 --root /opt/gnoblin-build-tools
fi
export PATH="/opt/gnoblin-build-tools/bin:$PATH"
cargo install --locked just --version 1.40.0 --root /opt/gnoblin-build-tools
just --version
id builder >/dev/null 2>&1 || useradd -m builder
install -d -o builder -g builder /usr/lib/gnoblin
printf '%s\n' 'Ready. As builder, with /opt/gnoblin-build-tools/bin on PATH, run scripts/build-deb.sh.'
