#!/usr/bin/env bash
# Build the private Gnoblin RPM dependency chain in a disposable Tumbleweed
# worker. The result is intentionally an artifact, never a package repository.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOPDIR="${1:?usage: $0 <rpmbuild-topdir>}"
TOPDIR="$(realpath -m "$TOPDIR")"
SOURCES="$TOPDIR/SOURCES"
BUILDROOT="$TOPDIR/BUILDROOT"

mkdir -p "$SOURCES" "$BUILDROOT"

"$ROOT/packaging/opensuse/check-buildrequires.sh" --install
git -C "$ROOT" submodule foreach --recursive 'git fetch --force --tags origin'
for project in gsettings-desktop-schemas mutter gnome-shell; do
    "$ROOT/scripts/make-tarball.sh" "$project" "$SOURCES"
done

build() {
    local spec="$1"
    shift
    rpmbuild -ba \
        --define "_topdir $TOPDIR" \
        --define "_sourcedir $SOURCES" \
        "$@" \
        "$ROOT/packaging/opensuse/$spec"
}

install_output() {
    local package
    for package in "$@"; do
        zypper --non-interactive install --no-recommends --allow-unsigned-rpm "$package"
    done
}

build gsettings-desktop-schemas.spec
mapfile -t schema_rpms < <(find "$TOPDIR/RPMS" -type f -name 'gnoblin-gsettings-desktop-schemas-*.rpm' | sort)
((${#schema_rpms[@]} == 1))
install_output "${schema_rpms[@]}"

build mutter.spec --with gnoblin_stack
mapfile -t mutter_rpms < <(find "$TOPDIR/RPMS" -type f \( -name 'gnoblin-mutter-[0-9]*.rpm' -o -name 'gnoblin-mutter-devel-[0-9]*.rpm' \) | sort)
((${#mutter_rpms[@]} == 2))
install_output "${mutter_rpms[@]}"

build gnome-shell.spec --with gnoblin_stack
build gnoblin.spec

find "$TOPDIR/RPMS" -type f -name '*.rpm' -print | LC_ALL=C sort
