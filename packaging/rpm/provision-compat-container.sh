#!/usr/bin/env bash
# Prepare an isolated RPM-family image to build the private GLib/GI bootstrap.
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this container provisioning step as root." >&2
    exit 1
fi

source /etc/os-release
case "${ID}:${VERSION_ID}" in
    rocky:8 | rocky:8.* | rhel:8 | rhel:8.* | almalinux:8 | almalinux:8.*)
        dnf -qy install \
            gcc gcc-c++ make pkgconf-pkg-config python3 python3-devel python3-pip \
            python3-setuptools python39 python39-devel python39-pip python39-setuptools \
            flex bison gettext libffi-devel pcre2-devel zlib-devel libselinux-devel \
            tar xz patch git
        python3.9 -m venv /opt/gnoblin-rpm-compat-tools
        /opt/gnoblin-rpm-compat-tools/bin/pip install --disable-pip-version-check --quiet setuptools meson==1.10.1 ninja
        ;;
    rocky:9 | rocky:9.* | rhel:9 | rhel:9.* | almalinux:9 | almalinux:9.* | \
        rocky:10 | rocky:10.* | rhel:10 | rhel:10.* | almalinux:10 | almalinux:10.*)
        dnf -qy install \
            gcc gcc-c++ make pkgconf-pkg-config python3 python3-devel python3-pip \
            python3-setuptools flex bison gettext libffi-devel pcre2-devel zlib-devel \
            libmount-devel libselinux-devel tar xz patch git
        python3 -m venv /opt/gnoblin-rpm-compat-tools
        /opt/gnoblin-rpm-compat-tools/bin/pip install --disable-pip-version-check --quiet setuptools meson==1.10.1 ninja
        ;;
    opensuse-leap:16.0)
        zypper --non-interactive --quiet install --no-recommends \
            gcc gcc-c++ make meson ninja pkgconf-pkg-config python3 python3-devel \
            python3-setuptools flex bison gettext-tools libffi-devel pcre2-devel zlib-devel \
            libmount-devel libselinux-devel tar xz patch git
        ;;
    opensuse-leap:15.6)
        zypper --non-interactive --quiet install --no-recommends \
            gcc gcc-c++ make pkg-config python3-devel flex bison gettext-tools \
            libffi-devel pcre2-devel zlib-devel libmount-devel libselinux-devel \
            libopenssl-devel sqlite3-devel tar gzip xz patch git curl
        python_archive=/tmp/Python-3.9.20.tgz
        curl --fail --location --silent --show-error \
            --output "$python_archive" https://www.python.org/ftp/python/3.9.20/Python-3.9.20.tgz
        echo '1e71f006222666e0a39f5a47be8221415c22c4dd8f25334cc41aee260b3d379e  /tmp/Python-3.9.20.tgz' | sha256sum --check --status
        tar -xzf "$python_archive" -C /tmp
        (cd /tmp/Python-3.9.20 && ./configure --prefix=/opt/gnoblin-python39 --with-ensurepip=install && make -j2 && make install)
        /opt/gnoblin-python39/bin/python3.9 -m venv /opt/gnoblin-rpm-compat-tools
        /opt/gnoblin-rpm-compat-tools/bin/pip install --disable-pip-version-check --quiet setuptools meson==1.10.1 ninja
        ;;
    *)
        echo "The private GLib/GI bootstrap gate currently supports EL 8/9/10 and openSUSE Leap 16.0 only; got ${ID}:${VERSION_ID}." >&2
        exit 2
        ;;
esac

id -u gnoblin-build >/dev/null 2>&1 || useradd --create-home --shell /bin/bash gnoblin-build
