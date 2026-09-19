#!/usr/bin/env bash
# Produce a release tarball of a subproject with gnoblin's changes applied.
#
# apply-patches.sh materialises the owned overlays and patch series. The archive
# manifest combines Git-tracked paths, pinned mandatory Meson subprojects, and
# Gnoblin's overlay destinations. Unrelated checkout state is excluded.
# Metadata and compression are normalised so identical source yields identical
# bytes, and publication happens only after sidecar staging succeeds.
set -euo pipefail

PROJ="${1:?usage: make-tarball.sh <mutter|gnome-shell|gsettings-desktop-schemas> [outdir]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"
OUTDIR="${2:-${HOME}/rpmbuild/SOURCES}"

# RPM Version field stays numeric; the gnoblin marker lives in Release/meson.
case "$PROJ" in
    mutter | gnome-shell | gsettings-desktop-schemas) VER="$($ROOT/scripts/gnome-versions.py get "$PROJ" version)" ;;
    *)
        echo "unknown subproject: $PROJ" >&2
        exit 1
        ;;
esac

if [[ "$PROJ" == gsettings-desktop-schemas ]]; then
    COMMIT="$($ROOT/scripts/gnome-versions.py get "$PROJ" commit)"
    WORK="$(mktemp -d)"
    cleanup_work() {
        rm -rf -- "$WORK"
    }
    trap cleanup_work EXIT
    git -C "$WORK" init -q
    git -C "$WORK" remote add origin https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas.git
    git -C "$WORK" fetch -q --depth=1 origin "$COMMIT"
    [[ "$(git -C "$WORK" rev-parse FETCH_HEAD)" == "$COMMIT" ]]
    EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$WORK" show -s --format=%ct FETCH_HEAD)}"
    mkdir -p "$OUTDIR"
    OUT="$OUTDIR/${PROJ}-${VER}.tar.xz"
    TEMP="$(mktemp --tmpdir="$OUTDIR" ".${PROJ}-${VER}.tar.xz.XXXXXX")"
    git -C "$WORK" archive --format=tar --prefix="${PROJ}-${VER}/" FETCH_HEAD |
        xz -T1 -9 >"$TEMP"
    touch --date="@$EPOCH" "$TEMP"
    chmod 0644 "$TEMP"
    mv -f -- "$TEMP" "$OUT"
    echo "$OUT"
    exit 0
fi

EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$SM" log -1 --format=%ct "$VER")}"
case "$EPOCH" in
    "" | *[!0-9]*)
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
echo ">> staging $PROJ RPM sidecar sources -> $OUTDIR" >&2
"$ROOT/scripts/stage-rpm-sources.sh" "$PROJ" "$OUTDIR"
mv -f -- "$TEMP" "$OUT"
trap - EXIT
echo "$OUT"
