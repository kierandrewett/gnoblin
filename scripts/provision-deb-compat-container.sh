#!/usr/bin/env bash
# Prepare a disposable old Debian-family builder for the complete private
# GNOME compatibility runtime and DEB package.
set -euo pipefail
export PATH="$PATH"
if [ "$(id -u)" -ne 0 ] || { [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; }; then
    echo 'This script only runs as root inside a disposable Docker/Podman container.' >&2
    exit 2
fi
source /etc/os-release
case "$ID:$VERSION_ID" in
    debian:11)
        # Debian 11 LTS ended in August 2026. Pin its final package set so
        # builds stay repeatable after the live security archive is retired.
        rm -f /etc/apt/sources.list.d/*
        cat >/etc/apt/sources.list <<'EOF'
deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260831T235959Z bullseye main
deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260831T235959Z bullseye-updates main
deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260831T235959Z bullseye-security main
EOF
        ;;
    debian:12 | ubuntu:22.04) ;;
    *)
        echo 'Private compatibility builds target Debian 11/12 and Ubuntu 22.04 only.' >&2
        exit 2
        ;;
esac
source scripts/build-deps.sh
install_build_dependencies debian true false true true
apt-get install -y --no-install-recommends \
    bison ca-certificates clang cbindgen flex gnupg libdrm-dev libegl-dev libepoxy-dev \
    libffi-dev libgbm-dev libgcrypt20-dev libgles-dev libgraphene-1.0-dev libheif-dev libseccomp-dev \
    libjpeg-dev libmount-dev libp11-kit-dev libpango1.0-dev \
    libpcre2-dev libpng-dev libsecret-1-dev libtiff-dev libwebp-dev \
    libxkbcommon-dev libxml2-dev llvm openssh-client python3-dev python3-venv zlib1g-dev
python3 -m venv --system-site-packages /opt/gnoblin-compat-build-tools
/opt/gnoblin-compat-build-tools/bin/pip install meson==1.10.1
python3 scripts/bootstrap-compat-rust.py --prefix /opt/gnoblin-compat-build-tools
export PATH="/opt/gnoblin-compat-build-tools/bin:$PATH"
# Mozilla's JavaScript engine needs cbindgen 0.27 or newer.  Debian 11/12
# and Ubuntu 22.04 ship an older package, so install this build-only tool next
# to the pinned Rust compiler instead of using the host executable.
CARGO_HOME=/opt/gnoblin-compat-build-tools/cargo-home \
    /opt/gnoblin-compat-build-tools/bin/cargo install cbindgen \
    --version 0.28.0 --locked --root /opt/gnoblin-compat-build-tools
CARGO_HOME=/opt/gnoblin-compat-build-tools/cargo-home \
    /opt/gnoblin-compat-build-tools/bin/cargo install just \
    --version 1.40.0 --locked --root /opt/gnoblin-compat-build-tools
/opt/gnoblin-compat-build-tools/bin/just --version
id builder >/dev/null 2>&1 || useradd -m builder
install -d -o builder -g builder /usr/lib/gnoblin /usr/lib/gnoblin/deps
printf '%s\n' 'Ready to build the pinned private GNOME compatibility runtime as builder.'
