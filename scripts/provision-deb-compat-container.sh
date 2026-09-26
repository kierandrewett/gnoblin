#!/usr/bin/env bash
# Prepare a disposable Debian 12 / Ubuntu 22.04 builder for the experimental
# private GTK/GCR dependency closure. This does not install a Gnoblin package.
set -euo pipefail
export PATH="$PATH"
if [ "$(id -u)" -ne 0 ] || { [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; }; then
    echo 'This script only runs as root inside a disposable Docker/Podman container.' >&2
    exit 2
fi
source /etc/os-release
case "$ID:$VERSION_ID" in
    debian:12 | ubuntu:22.04) ;;
    *)
        echo 'Experimental compatibility builds target Debian 12 and Ubuntu 22.04 only.' >&2
        exit 2
        ;;
esac
apt-get update
apt-get install -y --no-install-recommends \
    build-essential bison ca-certificates flex git libdrm-dev libegl-dev libepoxy-dev \
    libffi-dev libgbm-dev libgcrypt20-dev libgles-dev libgraphene-1.0-dev libheif-dev libseccomp-dev \
    libgtk-4-dev libjpeg-dev libmount-dev libp11-kit-dev libpango1.0-dev \
    libpcre2-dev libpng-dev libsecret-1-dev libtiff-dev libwayland-dev \
    libwebp-dev libxkbcommon-dev libxml2-dev make meson ninja-build openssh-client \
    gnupg pkg-config python-is-python3 python3 python3-dev python3-pip python3-venv \
    shared-mime-info wayland-protocols zlib1g-dev
python3 -m venv --system-site-packages /opt/gnoblin-compat-build-tools
/opt/gnoblin-compat-build-tools/bin/pip install meson==1.10.1
python3 scripts/bootstrap-compat-rust.py --prefix /opt/gnoblin-compat-build-tools
id builder >/dev/null 2>&1 || useradd -m builder
install -d -o builder -g builder /usr/lib/gnoblin /usr/lib/gnoblin/deps
printf '%s\n' 'Ready to build the experimental pinned GTK/GCR closure as builder.'
