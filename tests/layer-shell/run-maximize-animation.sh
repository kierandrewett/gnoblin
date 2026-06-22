#!/usr/bin/env bash
# Regression: maximize has a configurable frame-lerp animation.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== maximize must animate from restored frame to maximized frame =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/maximize-animation-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
