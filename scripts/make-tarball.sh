#!/usr/bin/env bash
# Produce a release tarball from the pinned Gnoblin Mutter fork or patched portal source.
#
# The archive manifest combines Git-tracked paths, pinned mandatory Meson
# subprojects, and any portal overlay destinations. Unrelated checkout state is excluded.
# Metadata and compression are normalised so identical source yields identical
# bytes. RPM-specific sidecar sources are staged when building RPMs.
set -euo pipefail

PROJ="${1:?usage: make-tarball.sh <mutter|xdg-desktop-portal-gnome> [outdir]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"
OUTDIR="${2:-${HOME}/rpmbuild/SOURCES}"

# RPM Version field stays numeric; the gnoblin marker lives in Release/meson.
case "$PROJ" in
    mutter | xdg-desktop-portal-gnome) VER="$($ROOT/scripts/gnome-versions.py get "$PROJ" version)" ;;
    *)
        echo "unknown subproject: $PROJ" >&2
        exit 1
        ;;
esac

EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$SM" log -1 --format=%ct "$VER")}"
case "$EPOCH" in
    "" | *[!0-9]*)
        echo "invalid SOURCE_DATE_EPOCH: $EPOCH" >&2
        exit 2
        ;;
esac

if [ "$PROJ" = mutter ]; then
    "$ROOT/scripts/ensure-release-subprojects.sh" mutter
    "$ROOT/scripts/subproject-state.sh" check mutter "$VER"
else
    "$ROOT/scripts/apply-patches.sh" "$PROJ" >&2
fi

mkdir -p "$OUTDIR"
ARCHIVE_NAME="$PROJ"
OUT="$OUTDIR/${ARCHIVE_NAME}-${VER}.tar.xz"
TEMP="$(mktemp --tmpdir="$OUTDIR" ".${ARCHIVE_NAME}-${VER}.tar.xz.XXXXXX")"
cleanup() {
    rm -f -- "$TEMP"
}
trap cleanup EXIT

echo ">> archiving $PROJ working tree -> $OUT" >&2
"$ROOT/scripts/list-tarball-sources.sh" "$PROJ" --prepare |
    LC_ALL=C sort -z -u |
    tar -C "$SM" \
        --sort=name \
        --format=posix \
        --mtime="@$EPOCH" \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        --pax-option=delete=atime,delete=ctime \
        --mode='a=rX,u+w' \
        --null \
        --verbatim-files-from \
        --no-recursion \
        --files-from=- \
        --transform="s,^,${PROJ}-${VER}/,SH" \
        --use-compress-program='xz -T1 -9' \
        -cf "$TEMP"

chmod 0644 "$TEMP"
mv -f -- "$TEMP" "$OUT"
trap - EXIT
echo "$OUT"
