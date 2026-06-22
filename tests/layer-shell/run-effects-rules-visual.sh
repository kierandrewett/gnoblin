#!/usr/bin/env bash
# Regression: the rules-based effects system renders distinct corner shapes
# (circular vs macOS squircle) and borders (coloured line + macOS raised lip).
# Drives a headless gnoblin-shell via the harness, spawns foot under various
# [effects]/[window-rules] configs, and pixel-checks the result. Needs a dev
# build + grim + foot + PIL + numpy.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL, numpy" 2>/dev/null || { echo "SKIP: needs python gi + PIL + numpy"; exit 0; }

echo "== rules-based effects: squircle/circular + line/lip borders (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/effects-rules-visual.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
