#!/usr/bin/env bash
# Create a reproducible source archive for the pinned GSettings schema input.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="$(realpath -m "${1:-$ROOT/dist/release}")"
VERSION="$("$ROOT/scripts/gnome-versions.py" get gsettings-desktop-schemas version)"
REVISION="$("$ROOT/scripts/gnome-versions.py" get gsettings-desktop-schemas commit)"
EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" show -s --format=%ct HEAD)}"
TEMP="$(mktemp -d)"
cleanup() {
    rm -rf -- "$TEMP"
}
trap cleanup EXIT

case "$EPOCH" in
    "" | *[!0-9]*)
        echo "invalid SOURCE_DATE_EPOCH: $EPOCH" >&2
        exit 2
        ;;
esac

git -C "$TEMP" init --quiet source
git -C "$TEMP/source" remote add origin https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas.git
git -C "$TEMP/source" fetch --quiet --depth=1 origin "$REVISION"
git -C "$TEMP/source" checkout --quiet --detach FETCH_HEAD
ACTUAL="$(git -C "$TEMP/source" rev-parse HEAD)"
if [ "$ACTUAL" != "$REVISION" ]; then
    echo "gsettings-desktop-schemas resolved to $ACTUAL, expected $REVISION" >&2
    exit 1
fi

TOP="gsettings-desktop-schemas-$VERSION"
mkdir -p "$TEMP/$TOP"
git -C "$TEMP/source" archive --format=tar "$REVISION" | tar -xf - -C "$TEMP/$TOP"
printf '%s\n' "$REVISION" >"$TEMP/$TOP/GNOBLIN_SOURCE_REVISION"
mkdir -p "$OUTPUT_DIR"
tar -C "$TEMP" \
    --sort=name \
    --format=posix \
    --mtime="@$EPOCH" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --pax-option=delete=atime,delete=ctime \
    --mode='a=rX,u+w' \
    --use-compress-program='xz -T1 -9' \
    -cf "$OUTPUT_DIR/gsettings-desktop-schemas-$VERSION.tar.xz" "$TOP"
