#!/usr/bin/env bash
# Regression: the [effects] blur-alpha-threshold knob gates the frost by the
# surface's own alpha (frost only where alpha < threshold). Drives a headless
# gnoblin-shell. Needs a dev build + foot (with colors.alpha) + PIL/numpy.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL, numpy" 2>/dev/null || { echo "SKIP: needs python gi/PIL/numpy"; exit 0; }

echo "== blur-alpha-threshold gates the frost by surface alpha (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/blur-alpha-threshold-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
