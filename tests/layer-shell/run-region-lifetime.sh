#!/usr/bin/env bash
# Regression: Slint layer-shell input-region updates must not leak wl_region
# objects. Runs notifyd with WAYLAND_DEBUG=client and checks create/destroy pairs.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v notify-send >/dev/null || { echo "SKIP: no notify-send"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

echo "== Slint layer-shell input regions must destroy temporary wl_region objects =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/region-lifetime-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
