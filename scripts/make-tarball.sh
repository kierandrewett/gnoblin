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
echo "$OUT"

# The gnoblin-session subpackage (packaging/rpm/gnome-shell.spec) needs these
# as individual Source1..N files alongside the tarball -- they live at the
# gnoblin repo root (src/data/session/, src/tools/), not inside the
# gnome-shell submodule, so they can't ride along in the archive above.
if [ "$PROJ" = "gnome-shell" ]; then
  echo ">> staging gnoblin-session sources -> $OUTDIR" >&2
  cp "$ROOT/src/data/session/modes/gnoblin.json" "$OUTDIR/gnoblin.json"
  cp "$ROOT/src/data/session/gnome-session/gnoblin.session" "$OUTDIR/gnoblin.session"
  cp "$ROOT/src/data/session/gnoblin.desktop" "$OUTDIR/gnoblin.desktop"
  cp "$ROOT/src/data/session/systemd-user/org.gnoblin.Shell.target" "$OUTDIR/org.gnoblin.Shell.target"
  cp "$ROOT/src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in" \
     "$OUTDIR/org.gnoblin.Shell@wayland.service.in"
  cp "$ROOT/src/tools/gnoblin-env.sh" "$OUTDIR/gnoblin-env.sh"
  cp "$ROOT/src/tools/gnoblin-session" "$OUTDIR/gnoblin-session"
  cp "$ROOT/src/tools/gnoblin-shell-service" "$OUTDIR/gnoblin-shell-service"
  cp "$ROOT/src/tools/gnoblinctl" "$OUTDIR/gnoblinctl"
fi
