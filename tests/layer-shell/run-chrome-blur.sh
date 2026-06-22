#!/usr/bin/env bash
# Regression: gnoblin's own layer-shell chrome (topbar/dock/...) is frosted by the
# compositor by default (depends on the mutter layer-shell namespace stash + the
# content-behind blur). Drives a headless gnoblin-shell WITH clients via the
# harness. Needs a full dev build (mutter + shell + clients) + foot.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-topbar" ] || { echo "SKIP: no gnoblin-topbar (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL, numpy" 2>/dev/null || { echo "SKIP: needs python gi/PIL/numpy"; exit 0; }

echo "== default compositor frost on gnoblin chrome (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/chrome-blur-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
