#!/usr/bin/env bash
# Regression: touchpad gestures -> action dispatcher. Real multitouch can't be
# injected headlessly, so this drives the resolution path via `dispatch gesture
# <key>` and asserts [gestures] override / default / none behaviour. Needs a dev
# build.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== touchpad gestures -> actions (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/gestures-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
