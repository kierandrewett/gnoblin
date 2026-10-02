#!/usr/bin/env bash
# Build the private Gnoblin RPM dependency chain in a disposable Tumbleweed
# worker. The result is intentionally an artifact, never a package repository.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOPDIR="${1:?usage: $0 <rpmbuild-topdir> [prepared-source-directory]}"
TOPDIR="$(realpath -m "$TOPDIR")"
SOURCES="$TOPDIR/SOURCES"
BUILDROOT="$TOPDIR/BUILDROOT"
PREPARED_SOURCES="${2:-}"

mkdir -p "$SOURCES" "$BUILDROOT"

# The source archiver needs the host tools from the first build stage too.
"$ROOT/packaging/opensuse/check-buildrequires.sh" mutter --install

if [[ -n "$PREPARED_SOURCES" ]]; then
    gnoblin_version="$("$ROOT/scripts/gnoblin-version.py" get version)"
    gnoblin_source="$PREPARED_SOURCES/gnoblin-$gnoblin_version-source.tar.xz"
    [[ -f "$gnoblin_source" ]] || {
        echo "Missing complete Gnoblin source bundle: $gnoblin_source" >&2
        exit 1
    }
    for project in mutter xdg-desktop-portal-gnome; do
        version="$($ROOT/scripts/gnome-versions.py get "$project" version)"
        source="$PREPARED_SOURCES/$project-$version.tar.xz"
        [[ -f "$source" ]] || {
            echo "Missing prepared source: $source" >&2
            exit 1
        }
        install -m 0644 -- "$source" "$SOURCES/"
    done
else
    git -C "$ROOT" submodule foreach --recursive 'git fetch --force --tags origin'
    for project in mutter xdg-desktop-portal-gnome; do
        "$ROOT/scripts/make-tarball.sh" "$project" "$SOURCES"
    done
fi
"$ROOT/scripts/stage-rpm-sources.sh" mutter "$SOURCES"

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

build mutter.spec
mapfile -t mutter_rpms < <(find "$TOPDIR/RPMS" -type f \( -name 'gnoblin-mutter-[0-9]*.rpm' -o -name 'gnoblin-mutter-devel-[0-9]*.rpm' \) | sort)
((${#mutter_rpms[@]} == 2))
install_output "${mutter_rpms[@]}"

"$ROOT/packaging/opensuse/check-buildrequires.sh" gnoblin-portal --install
build gnoblin-portal.spec
"$ROOT/packaging/opensuse/check-buildrequires.sh" gnoblin --install
build gnoblin.spec

find "$TOPDIR/RPMS" -type f -name '*.rpm' -print | LC_ALL=C sort
