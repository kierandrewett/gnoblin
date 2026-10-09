#!/usr/bin/env bash
# Produce a release tarball from the patched Mutter or portal source. It is the same tree that ./build.sh compiles.
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

# Both components take the same path as ./build.sh: reset to the upstream tag, copy the overlay and apply patches/<project>/.
# The release tarball therefore holds exactly the tree a source build compiles, and no commit has to be published to the
# Mutter fork for it. apply-patches.sh checks that the checkout is pristine or in a state it recorded.
if [ "$PROJ" = mutter ]; then
    "$ROOT/scripts/ensure-release-subprojects.sh" mutter
fi
"$ROOT/scripts/apply-patches.sh" "$PROJ" >&2

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
