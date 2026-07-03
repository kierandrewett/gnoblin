#!/usr/bin/env bash
# Prepare a submodule checkout with gnoblin's changes for building.
#
# The submodules under subprojects/ are always kept at their pinned upstream
# tag. gnoblin changes come from two places, applied here at build time:
#   1. overlay source files (large new files) copied in via copy-overlay.sh
#   2. patches (edits to existing files / small additions) applied with git am
# This script resets the submodule to its tag first, so it is idempotent and
# never accumulates state.
set -euo pipefail

PROJ="${1:?usage: apply-patches.sh <mutter|gnome-shell>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"

case "$PROJ" in
  mutter)                       TAG="49.5" ;;
  gnome-shell)                  TAG="49.6" ;;
  xdg-desktop-portal-gnome)     TAG="49.0" ;;
  *) echo "unknown subproject: $PROJ" >&2; exit 1 ;;
esac

git -C "$SM" rev-parse --git-dir >/dev/null 2>&1 \
  || { echo "submodule $PROJ not initialised; run 'just init'" >&2; exit 1; }

echo ">> resetting $PROJ to pristine tag $TAG"
git -C "$SM" am --abort >/dev/null 2>&1 || true
git -C "$SM" checkout -qf "$TAG"
git -C "$SM" reset -q --hard "$TAG"
git -C "$SM" clean -qfdx

# 1. Copy overlay source files (new files we author) into the submodule.
"$ROOT/scripts/copy-overlay.sh" "$PROJ" "$SM"

# 2. Apply the patch series (edits to existing files).
mapfile -t PATCHES < <(find "$ROOT/patches/$PROJ" -name '*.patch' | sort)
echo ">> applying ${#PATCHES[@]} patch(es) to $PROJ"
if [ "${#PATCHES[@]}" -gt 0 ]; then
  printf '   %s\n' "${PATCHES[@]#$ROOT/}"
  # Apply with your ambient git identity and config unchanged (no -c overrides,
  # no signing flags). The patch authorship comes from each file's From: header.
  # The .patch files under patches/ are the source of truth; these commits are
  # only a staging step before `git archive`/tarball and are never pushed.
  git -C "$SM" am "${PATCHES[@]}"
fi

echo ">> $PROJ now at $(git -C "$SM" rev-parse --short HEAD) ($(git -C "$SM" log --oneline "$TAG..HEAD" | wc -l) patches on top of $TAG)"
