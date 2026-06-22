#!/usr/bin/env bash
# Regression: drop-shadows must NOT be blurred into the background-blur frost.
# A floating window's shadow must stay crisp on the wallpaper and out of the
# frosted gnoblin dock that sits over it. Drives a headless gnoblin-shell.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL, numpy" 2>/dev/null || { echo "SKIP: needs python gi/PIL/numpy"; exit 0; }

echo "== drop-shadows stay out of the background-blur frost (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/shadow-not-blurred-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
