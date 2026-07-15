#!/usr/bin/env bash
# Produce a release tarball of a subproject with gnoblin's changes applied.
#
# apply-patches.sh copies in the overlay source files (untracked) and applies
# the patch series, so the tarball is archived from the WORKING TREE (not
# `git archive HEAD`, which would omit the untracked overlay files). The result
# is pristine-upstream + gnoblin overlay + patches, with NO Patch: directives
# needed in the RPM/deb spec.
set -euo pipefail

PROJ="${1:?usage: make-tarball.sh <mutter|gnome-shell> [outdir]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"
OUTDIR="${2:-${HOME}/rpmbuild/SOURCES}"

# RPM Version field stays numeric; the gnoblin marker lives in Release/meson.
case "$PROJ" in
  mutter)               VER="49.5" ;;
  gnome-shell)          VER="49.6" ;;
  *) echo "unknown subproject: $PROJ" >&2; exit 1 ;;
esac

"$ROOT/scripts/apply-patches.sh" "$PROJ" >&2

mkdir -p "$OUTDIR"
OUT="$OUTDIR/${PROJ}-${VER}.tar.xz"
echo ">> archiving $PROJ working tree -> $OUT" >&2
tar -C "$(dirname "$SM")" \
    --exclude='*/.git' \
    --transform="s,^\./$PROJ,${PROJ}-${VER}," \
    --use-compress-program='xz -T0' \
    -cf "$OUT" "./$PROJ"
echo ">> staging $PROJ RPM sidecar sources -> $OUTDIR" >&2
"$ROOT/scripts/stage-rpm-sources.sh" "$PROJ" "$OUTDIR"
echo "$OUT"
