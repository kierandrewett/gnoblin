#!/usr/bin/env bash
# Build the reproducible Gnoblin source tarball before optional package assets.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! git -C "$ROOT" diff --ignore-submodules=all --quiet HEAD -- ||
    [ -n "$(git -C "$ROOT" ls-files --others --exclude-standard)" ]; then
    echo "Commit Gnoblin source changes before creating release assets; the bundle archives HEAD." >&2
    exit 2
fi
OUTPUT="$(realpath -m "${1:-$ROOT/dist/release}")"
RELEASE_TAG="${2:-}"
SOURCE_ONLY="${3:-}"
if [ -n "$SOURCE_ONLY" ] && [ "$SOURCE_ONLY" != --source-only ]; then
    echo "Unknown option: $SOURCE_ONLY" >&2
    exit 2
fi
GNOME_VERSION="$($ROOT/scripts/gnome-versions.py get mutter version)"
GNOBLIN_VERSION="$($ROOT/scripts/gnoblin-version.py get version)"
PUBLIC_RELEASE_TAG="${4:-gnoblin-v$GNOBLIN_VERSION}"
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
"$ROOT/scripts/make-tarball.sh" xdg-desktop-portal-gnome "$SOURCES"
install -m 0644 -- "$SOURCES/mutter-$GNOME_VERSION.tar.xz" "$OUTPUT/"
install -m 0644 -- "$SOURCES/xdg-desktop-portal-gnome-$GNOME_VERSION.tar.xz" "$OUTPUT/"
SOURCE_BUNDLE="$OUTPUT/gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION-source.tar.xz"
"$ROOT/scripts/build-source-bundle.sh" \
    "$SOURCE_BUNDLE" \
    "$GNOBLIN_VERSION" \
    "$SOURCES/mutter-$GNOME_VERSION.tar.xz" \
    "$SOURCES/xdg-desktop-portal-gnome-$GNOME_VERSION.tar.xz"
# The main source RPM consumes the same complete bundle under its Source0
# filename. Keep this alias in the private staging directory; the public
# release still publishes one versioned source bundle.
install -m 0644 -- "$SOURCE_BUNDLE" "$SOURCES/gnoblin-$GNOBLIN_VERSION-source.tar.xz"
SOURCE_BUNDLE_SHA256="$(sha256sum "$SOURCE_BUNDLE" | awk '{print $1}')"
python3 "$ROOT/scripts/sync-package-manifest.py" arch-release \
    --output "$OUTPUT/gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION.PKGBUILD" \
    --source-sha256 "$SOURCE_BUNDLE_SHA256" \
    --release-tag "$PUBLIC_RELEASE_TAG"
install -m 0644 -- "$ROOT/packaging/arch/gnome-integration/PKGBUILD" \
    "$OUTPUT/gnoblin-gnome-integration-$GNOBLIN_VERSION.PKGBUILD"

if [ "$SOURCE_ONLY" != --source-only ]; then
    "$ROOT/scripts/build-srpm.sh" mutter "$SOURCES" "$SRPMS"
    "$ROOT/scripts/build-srpm.sh" gnoblin-portal "$SOURCES" "$SRPMS"
    "$ROOT/scripts/build-srpm.sh" gnoblin "$SOURCES" "$SRPMS"
    find "$SRPMS" -maxdepth 1 -type f -name '*.src.rpm' -exec install -m 0644 -t "$OUTPUT" -- {} +
fi

(
    cd "$OUTPUT"
    find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\n' |
        LC_ALL=C sort |
        xargs sha256sum >SHA256SUMS
)

printf 'release assets for %s:\n' "$EXPECTED_TAG"
find "$OUTPUT" -maxdepth 1 -type f -printf '  %f\n' | LC_ALL=C sort
