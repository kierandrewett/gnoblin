#!/usr/bin/env bash
# Regenerate patches/mutter/40-gnoblin-protocols/0001-*.patch.
#
# gnoblin's extra wlr-/ext- Wayland protocols are registered through a single
# aggregated entry point (meta_gnoblin_init_protocols, an overlay file). This
# script regenerates the ONE wiring patch that:
#   * adds the aggregator + per-protocol sources to src/meson.build
#   * adds the vendored protocol XML names to the meson protocol list
#   * includes the aggregator header and calls meta_gnoblin_init_protocols()
#     once from meta_wayland_shell_init()
#
# Insertions are anchored on pristine-stable context (and use 1-line diff
# context) so the patch applies cleanly to the pinned tag on its own, and
# co-exists with the separate layer-shell and screencopy wiring patches.
#
# To add a protocol: drop its overlay under src/<feature>/ with a manifest,
# add its init call to the aggregator overlay, then add its source files to
# SOURCES and its XML basename to PROTOCOLS below and re-run this script.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SM="$ROOT/subprojects/mutter"
TAG=49.5
OUT="$ROOT/patches/mutter/40-gnoblin-protocols"
TMP="$(mktemp -d /tmp/gnoblin-protocol-patch.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
SOURCES_FILE="$TMP/sources.txt"
PROTOCOLS_FILE="$TMP/protocols.txt"

# Overlay sources copied into src/wayland/ (aggregator first, then per protocol).
SOURCES=(
  gnoblin-config.c
  gnoblin-config.h
  meta-gnoblin-protocols.c
  meta-gnoblin-protocols.h
  meta-wayland-idle-notify.c
  meta-wayland-idle-notify.h
  meta-wayland-foreign-toplevel-common.c
  meta-wayland-foreign-toplevel-common.h
  meta-wayland-foreign-toplevel-list.c
  meta-wayland-foreign-toplevel-list.h
  meta-wayland-foreign-toplevel-management.c
  meta-wayland-foreign-toplevel-management.h
  meta-wayland-gamma-control.c
  meta-wayland-gamma-control.h
  meta-wayland-output-power-management.c
  meta-wayland-output-power-management.h
  meta-wayland-data-control.c
  meta-wayland-data-control.h
)

# Vendored protocol XML basenames (loaded as 'private' from overlay protocol/).
PROTOCOLS=(
  ext-idle-notify-v1
  ext-foreign-toplevel-list-v1
  wlr-foreign-toplevel-management-unstable-v1
  wlr-gamma-control-unstable-v1
  wlr-output-power-management-unstable-v1
  ext-data-control-v1
)

"$ROOT/scripts/subproject-state.sh" check mutter "$TAG"

git -C "$SM" am --abort >/dev/null 2>&1 || true
git -C "$SM" checkout -qf "$TAG"
git -C "$SM" reset -q --hard "$TAG"
git -C "$SM" clean -qfd

meson="$SM/src/meson.build"
surface="$SM/src/wayland/meta-wayland-surface.c"

# 1. meson sources block — anchored after meta-wayland-shell-surface.h.
{
  echo "    # gnoblin: extra wlr-/ext- protocols (sources copied from gnoblin overlays"
  echo "    # at build time; registered via meta_gnoblin_init_protocols)"
  for s in "${SOURCES[@]}"; do echo "    'wayland/$s',"; done
} > "$SOURCES_FILE"
GNOBLIN_SOURCES_FILE="$SOURCES_FILE" perl -0pi -e '
  local $/; open(my $f, "<", $ENV{"GNOBLIN_SOURCES_FILE"}); my $blk = <$f>; close($f);
  s@(    '"'"'wayland/meta-wayland-shell-surface.h'"'"',\n)@$1$blk@;
' "$meson"

# 2. meson protocol list block — anchored after xdg-toplevel-tag.
{
  echo "    # gnoblin: vendored wlr-/ext- protocols (overlay src/wayland/protocol/)"
  for p in "${PROTOCOLS[@]}"; do echo "    ['$p', 'private', ],"; done
} > "$PROTOCOLS_FILE"
GNOBLIN_PROTOCOLS_FILE="$PROTOCOLS_FILE" perl -0pi -e '
  local $/; open(my $f, "<", $ENV{"GNOBLIN_PROTOCOLS_FILE"}); my $blk = <$f>; close($f);
  s@(    \['"'"'xdg-toplevel-tag'"'"', '"'"'staging'"'"', 1, \],\n)@$1$blk@;
' "$meson"

# 3. surface.c — aggregator include + single init call.
perl -0pi -e 's@(#include "wayland/meta-wayland-color-representation.h"\n)@$1#include "wayland/meta-gnoblin-protocols.h"\n@' "$surface"
perl -0pi -e 's@(  meta_wayland_xdg_shell_init \(compositor\);\n)@$1  meta_gnoblin_init_protocols (compositor);\n@' "$surface"

git -C "$SM" add src/meson.build src/wayland/meta-wayland-surface.c
git -C "$SM" commit -q -m "gnoblin-protocols: wire gnoblin's extra Wayland protocols into the build

Add a single aggregated entry point, meta_gnoblin_init_protocols(), called
once from meta_wayland_shell_init(), plus the meson sources and vendored
protocol XML entries for gnoblin's wlr-/ext- protocols.

The implementations, headers and protocol XML are gnoblin overlay files,
copied into src/wayland/ at build time. New protocols are added by editing
the aggregator overlay and scripts/gen-gnoblin-protocols-patch.sh, not this
patch by hand. Insertions are anchored on pristine-stable context so the
patch applies cleanly to the pinned tag independently of the layer-shell and
screencopy wiring patches."

mkdir -p "$OUT"
rm -f "$OUT"/*.patch
git -C "$SM" format-patch -1 HEAD --unified=1 -o "$OUT" >/dev/null

git -C "$SM" checkout -qf "$TAG"
git -C "$SM" reset -q --hard "$TAG"
git -C "$SM" clean -qfd

"$ROOT/scripts/subproject-state.sh" record mutter "$TAG"

echo ">> regenerated $(ls "$OUT"/*.patch)"
