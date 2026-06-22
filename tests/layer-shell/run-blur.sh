#!/usr/bin/env bash
# Regression: blur-behind (`[appearance] blur`). Boots a headless gnoblin-shell
# with blur on, opens the Overview, and asserts the Gaussian-blurred desktop
# clone composites behind the tint (the blur shader runs on llvmpipe too). Needs
# a dev build + grim + foot + PIL.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== blur-behind in the Overview (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/blur-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
