#!/usr/bin/env bash
# Regression: the app grid (`gnoblin-launcher --grid`). Drives a headless
# gnoblin-shell via the harness, asserts it renders as an icon grid (square
# selection tile, not a full-width row), closes on Escape, and launches a clicked
# app tile.
# Needs a dev build + grim + the launcher + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-launcher" ] || { echo "SKIP: gnoblin-launcher not built"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== app grid (gnoblin-launcher --grid, headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/grid-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
