#!/usr/bin/env bash
# Regenerate the gnoblin-protocols wiring patch in patches/mutter, found through its series.
#
# gnoblin's extra wlr-/ext- Wayland protocols are registered through a single
# aggregated entry point (meta_gnoblin_init_protocols, an overlay file). This
# script regenerates the ONE wiring patch that includes the aggregator header
# and calls meta_gnoblin_init_protocols() once from meta_wayland_shell_init().
#
# The protocol sources and XML names are not in this patch. They are listed in
# src/mutter-build/wiring.meson, an overlay that the one Meson include patch
# pulls in.
#
# The insertions are anchored on pristine-stable context (and use 1-line diff
# context) so the patch applies cleanly to the pinned tag on its own, and
# co-exists with the separate layer-shell and screencopy wiring patches.
#
# To add a protocol: drop its overlay under src/<feature>/ with a manifest, add
# its init call to the aggregator overlay, and list its source files and its XML
# basename in src/mutter-build/wiring.meson.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAG="$($ROOT/scripts/gnome-versions.py get mutter version)"
OUT="$("$ROOT/scripts/manage-patches.py" path mutter gnoblin-protocols-wire-gnoblin-s-extra-Wayland-proto)"
TMP="$(mktemp -d /tmp/gnoblin-protocol-patch.XXXXXX)"
WORKTREE="$TMP/mutter"
trap 'git -C "$ROOT/subprojects/mutter" worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true; rm -rf "$TMP"' EXIT

# This repository commonly carries in-progress Mutter work.  Build the wiring
# patch in an isolated detached worktree so regeneration never resets, cleans,
# or otherwise changes the developer's checkout.
git -C "$ROOT/subprojects/mutter" worktree add --quiet --detach "$WORKTREE" "$TAG"
SM="$WORKTREE"

surface="$SM/src/wayland/meta-wayland-surface.c"

# surface.c — aggregator include + single init call.
perl -0pi -e 's@(#include "wayland/meta-wayland-color-representation.h"\n)@$1#include "wayland/meta-gnoblin-protocols.h"\n@' "$surface"
perl -0pi -e 's@(  meta_wayland_xdg_shell_init \(compositor\);\n)@$1  meta_gnoblin_init_protocols (compositor);\n@' "$surface"

git -C "$SM" add src/wayland/meta-wayland-surface.c
"$ROOT/scripts/manage-patches.py" export-worktree mutter "$OUT" \
    --source-tree "$SM" --replace --unified 1 \
    --subject "gnoblin-protocols: wire Gnoblin's extra Wayland protocols into the build"

echo ">> regenerated ${OUT#"$ROOT"/}"
