#!/usr/bin/env bash
# Regression: shell-ui full-screen clients must use the configured surface size
# before wl_output metadata settles; otherwise on-demand menus center as if the
# monitor were 800px tall.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-power-menu" ] || { echo "SKIP: no gnoblin-power-menu at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }

echo "== power menu must center on non-800px outputs (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/power-menu-output-size-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
