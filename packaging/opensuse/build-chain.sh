#!/usr/bin/env bash
# Build the private Gnoblin RPM dependency chain in a disposable Tumbleweed
# worker. The result is intentionally an artifact, never a package repository.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOPDIR="${1:?usage: $0 <rpmbuild-topdir>}"
TOPDIR="$(realpath -m "$TOPDIR")"
SOURCES="$TOPDIR/SOURCES"
BUILDROOT="$TOPDIR/BUILDROOT"
compatibility_runtime=${GNOBLIN_COMPAT_RUNTIME:-0}

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

build_compatibility_runtime() {
    [[ $compatibility_runtime == 1 ]] || return
    "$ROOT/packaging/rpm/provision-compat-container.sh"
    install -d -o gnoblin-build -g gnoblin-build /usr/lib/gnoblin
    chown -R gnoblin-build:gnoblin-build "$ROOT/build"
    runuser -u gnoblin-build -- \
        env GNOBLIN_BUILD_JOBS="${GNOBLIN_BUILD_JOBS:-2}" \
        "$ROOT/packaging/rpm/build-compat-runtime.sh"
    tar -C /usr/lib/gnoblin -cJf "$SOURCES/gnoblin-compat-runtime-51.0.tar.xz" deps
    build compat-runtime.spec
    mapfile -t compat_rpms < <(find "$TOPDIR/RPMS" -type f -name 'gnoblin-compat-runtime-[0-9]*.rpm' | sort)
    ((${#compat_rpms[@]} == 1))
    install_output "${compat_rpms[@]}"
}

build_compatibility_runtime
compat_args=()
if [[ $compatibility_runtime == 1 ]]; then
    compat_args=(--with gnoblin_compat_runtime)
fi
build gsettings-desktop-schemas.spec
mapfile -t schema_rpms < <(find "$TOPDIR/RPMS" -type f -name 'gnoblin-gsettings-desktop-schemas-*.rpm' | sort)
((${#schema_rpms[@]} == 1))
install_output "${schema_rpms[@]}"

build mutter.spec --with gnoblin_stack "${compat_args[@]}"
mapfile -t mutter_rpms < <(find "$TOPDIR/RPMS" -type f \( -name 'gnoblin-mutter-[0-9]*.rpm' -o -name 'gnoblin-mutter-devel-[0-9]*.rpm' \) | sort)
((${#mutter_rpms[@]} == 2))
install_output "${mutter_rpms[@]}"

build gnome-shell.spec --with gnoblin_stack "${compat_args[@]}"
build gnoblin.spec

find "$TOPDIR/RPMS" -type f -name '*.rpm' -print | LC_ALL=C sort
