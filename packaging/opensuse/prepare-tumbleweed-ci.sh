#!/usr/bin/env bash
# Prepare an ephemeral GitHub Actions container against the current Tumbleweed
# repository snapshot before resolving package dependencies.
set -euo pipefail

if (($# == 0)); then
    echo "Usage: $0 <package> [package ...]" >&2
    exit 2
fi

sed -i 's#http://#https://#g' /etc/zypp/repos.d/*.repo
zypper --non-interactive refresh

# These packages are OBS bootstrap artifacts, not valid providers for a normal
# Tumbleweed build environment. OBS itself excludes their build-only provides.
zypper --non-interactive addlock glib2-stage1-devel libudev-mini1

# The published container can lag the rolling repositories. Keep exact runtime
# dependencies such as PCRE2 on one repository snapshot before installing the
# build or verification tools.
zypper --non-interactive dist-upgrade --allow-downgrade --no-recommends

# The container image carries this BusyBox applet, which conflicts with the
# full gawk required by rpm-build and is excluded by OBS's Tumbleweed policy.
if rpm -q busybox-gawk >/dev/null 2>&1; then
    zypper --non-interactive remove busybox-gawk
fi

zypper --non-interactive install --no-recommends "$@"
