#!/usr/bin/env bash
# Regression: [window-rules] place/state windows at map time, and the new WM
# dispatcher actions work. Drives a headless gnoblin-shell via the harness.
# Needs a dev build + foot.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

echo "== window rules apply at map + new actions dispatch (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/window-rules-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
