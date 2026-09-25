#!/usr/bin/env bash
# Build the self-contained source bundle consumed by the released Arch recipe.
#
# The public GitHub source archive is insufficient: it omits Git submodules and
# therefore cannot reproduce Gnoblin's patched Mutter and GNOME Shell.  The
# three component archives passed here are already materialised by
# scripts/make-tarball.sh and include Gnoblin's patch stacks.
set -euo pipefail

if [ "$#" -ne 5 ]; then
    echo "Usage: $0 <output> <gnoblin-version> <schemas.tar.xz> <mutter.tar.xz> <shell.tar.xz>" >&2
    exit 2
fi

OUTPUT="$(realpath -m "$1")"
VERSION="$2"
SCHEMAS="$(realpath "$3")"
MUTTER="$(realpath "$4")"
SHELL="$(realpath "$5")"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" show -s --format=%ct HEAD)}"
STAGING="$(mktemp -d)"
cleanup() {
    rm -rf -- "$STAGING"
}
trap cleanup EXIT

for source in "$SCHEMAS" "$MUTTER" "$SHELL"; do
    [ -f "$source" ] || {
        echo "missing prepared component source: $source" >&2
        exit 1
    }
done

TOP="gnoblin-$VERSION"
mkdir -p "$STAGING/$TOP/sources"
git -C "$ROOT" archive --format=tar HEAD | tar -xf - -C "$STAGING/$TOP"
install -m 0644 -- "$SCHEMAS" "$STAGING/$TOP/sources/"
install -m 0644 -- "$MUTTER" "$STAGING/$TOP/sources/"
install -m 0644 -- "$SHELL" "$STAGING/$TOP/sources/"

mkdir -p "$(dirname "$OUTPUT")"
tar -C "$STAGING" \
    --sort=name \
    --format=posix \
    --mtime="@$EPOCH" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --pax-option=delete=atime,delete=ctime \
    --mode='a=rX,u+w' \
    --use-compress-program='xz -T1 -9' \
    -cf "$OUTPUT" "$TOP"

echo "$OUTPUT"
