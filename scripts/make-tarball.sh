#!/usr/bin/env bash
# Produce a release tarball of a subproject with gnoblin's changes applied.
#
# apply-patches.sh copies in the owned overlay files and applies the patch
# series. The archive manifest combines Git-tracked paths with the destination
# paths from those overlay manifests; unrelated and ignored checkout state is
# excluded. Metadata and compression are normalised so identical source yields
# identical bytes, and publication happens only after sidecar staging succeeds.
set -euo pipefail

PROJ="${1:?usage: make-tarball.sh <mutter|gnome-shell> [outdir]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"
OUTDIR="${2:-${HOME}/rpmbuild/SOURCES}"

# RPM Version field stays numeric; the gnoblin marker lives in Release/meson.
case "$PROJ" in
    mutter) VER="49.5" ;;
    gnome-shell) VER="49.6" ;;
    *) echo "unknown subproject: $PROJ" >&2; exit 1 ;;
esac
EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$SM" log -1 --format=%ct "$VER")}"
case "$EPOCH" in
    ""|*[!0-9]*)
        echo "invalid SOURCE_DATE_EPOCH: $EPOCH" >&2
        exit 2
        ;;
esac

"$ROOT/scripts/apply-patches.sh" "$PROJ" >&2

mkdir -p "$OUTDIR"
OUT="$OUTDIR/${PROJ}-${VER}.tar.xz"
TEMP="$(mktemp --tmpdir="$OUTDIR" ".${PROJ}-${VER}.tar.xz.XXXXXX")"
cleanup() {
    rm -f -- "$TEMP"
}
trap cleanup EXIT

echo ">> archiving $PROJ working tree -> $OUT" >&2
{
    git -C "$SM" ls-files --cached -z
    "$ROOT/scripts/copy-overlay.sh" "$PROJ" "$SM" --list-destinations
} | LC_ALL=C sort -z -u |
    tar -C "$SM" \
        --sort=name \
        --format=posix \
        --mtime="@$EPOCH" \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        --pax-option=delete=atime,delete=ctime \
        --null \
        --verbatim-files-from \
        --no-recursion \
        --files-from=- \
        --transform="s,^,${PROJ}-${VER}/," \
        --use-compress-program='xz -T1 -9' \
        -cf "$TEMP"

chmod 0644 "$TEMP"
echo ">> staging $PROJ RPM sidecar sources -> $OUTDIR" >&2
"$ROOT/scripts/stage-rpm-sources.sh" "$PROJ" "$OUTDIR"
mv -f -- "$TEMP" "$OUT"
trap - EXIT
echo "$OUT"
