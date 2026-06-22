#!/usr/bin/env bash
# Regression: full-screen OSD overlay must not intercept pointer input.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-topbar" ] || { echo "SKIP: gnoblin-topbar not built"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-osd" ] || { echo "SKIP: gnoblin-osd not built"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0 (needed for pointer injection)"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

echo "== OSD overlay must pass pointer input through to the topbar =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/osd-passthrough-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
