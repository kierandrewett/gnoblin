#!/usr/bin/env bash
# Install the packages that PKGBUILD files declare, and nothing else.
#
# usage: install-arch-build-deps.sh PKGBUILD...
#
# CI used to keep its own list of Arch packages, and that list drifted from the recipes: a package missing from a
# PKGBUILD still built in CI and failed for users. Installing from the recipes makes CI fail the way a user would.
# Run as root in an Arch container. It installs the makedepends and depends of every recipe given. pacman resolves
# group names such as base-devel and version constraints such as glib2>=2.86.0.
set -euo pipefail

if [ "$#" -eq 0 ]; then
    echo "usage: $0 PKGBUILD..." >&2
    exit 2
fi

packages=()
for pkgbuild in "$@"; do
    [ -f "$pkgbuild" ] || {
        echo "[arch-deps] missing recipe: $pkgbuild" >&2
        exit 1
    }
    # A PKGBUILD is a shell file that only declares data and functions, so sourcing it in a subshell is safe.
    mapfile -t declared < <(bash -c 'source "$1"; printf "%s\n" "${makedepends[@]}" "${depends[@]}"' bash "$pkgbuild")
    echo "[arch-deps] $pkgbuild: ${#declared[@]} declared packages"
    packages+=("${declared[@]}")
done

mapfile -t unique < <(printf '%s\n' "${packages[@]}" | LC_ALL=C sort -u)
pacman -Syu --noconfirm --needed "${unique[@]}"
