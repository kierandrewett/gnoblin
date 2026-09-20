#!/usr/bin/env bash
# Build the reproducible source archives and source RPMs for one Gnoblin release.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="$(realpath -m "${1:-$ROOT/dist/release}")"
RELEASE_TAG="${2:-}"
GNOME_VERSION="$($ROOT/scripts/gnome-versions.py get mutter version)"
GNOBLIN_VERSION="$($ROOT/scripts/gnoblin-version.py get version)"
EXPECTED_TAG="gnoblin-v$GNOBLIN_VERSION"

if [[ -n "$RELEASE_TAG" ]]; then
    "$ROOT/scripts/check-release-tag.sh" "$RELEASE_TAG" >/dev/null
fi

if [[ -e "$OUTPUT" ]] && [[ -n "$(find "$OUTPUT" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    echo "release output directory is not empty: $OUTPUT" >&2
    exit 2
fi

WORK="$(mktemp -d)"
cleanup() {
    rm -rf -- "$WORK"
}
trap cleanup EXIT

SOURCES="$WORK/sources"
SRPMS="$WORK/srpms"
mkdir -p "$OUTPUT" "$SOURCES" "$SRPMS"

"$ROOT/scripts/make-tarball.sh" mutter "$SOURCES"
"$ROOT/scripts/make-tarball.sh" gnome-shell "$SOURCES"
"$ROOT/scripts/make-tarball.sh" gsettings-desktop-schemas "$SOURCES"
"$ROOT/scripts/build-srpm.sh" gsettings-desktop-schemas "$SOURCES" "$SRPMS"
"$ROOT/scripts/build-srpm.sh" mutter "$SOURCES" "$SRPMS"
"$ROOT/scripts/build-srpm.sh" gnome-shell "$SOURCES" "$SRPMS"
"$ROOT/scripts/build-srpm.sh" gnoblin "$SOURCES" "$SRPMS"

install -m 0644 -- "$SOURCES/mutter-$GNOME_VERSION.tar.xz" "$OUTPUT/"
install -m 0644 -- "$SOURCES/gnome-shell-$GNOME_VERSION.tar.xz" "$OUTPUT/"
install -m 0644 -- "$SOURCES/gsettings-desktop-schemas-$GNOME_VERSION.tar.xz" "$OUTPUT/"
find "$SRPMS" -maxdepth 1 -type f -name '*.src.rpm' -exec install -m 0644 -t "$OUTPUT" -- {} +
install -m 0644 -- "$ROOT/packaging/arch/PKGBUILD" "$OUTPUT/gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION.PKGBUILD"
git -C "$ROOT" archive --format=tar HEAD | xz >"$OUTPUT/gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION-debian.tar.xz"

(
    cd "$OUTPUT"
    find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\n' |
        LC_ALL=C sort |
        xargs sha256sum >SHA256SUMS
)

printf 'release assets for %s:\n' "$EXPECTED_TAG"
find "$OUTPUT" -maxdepth 1 -type f -printf '  %f\n' | LC_ALL=C sort
