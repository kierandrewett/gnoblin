#!/usr/bin/env bash
# Regression: named workspaces (WorkspaceNames D-Bus) + the scratchpad
# (scratchpad-stash / scratchpad toggle). Drives a headless gnoblin-shell via the
# harness. Needs a dev build + foot.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot (test needs a wayland app)"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== named workspaces + scratchpad (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/workspace-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
