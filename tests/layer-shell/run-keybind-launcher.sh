#!/usr/bin/env bash
# Interactive keybind regression: drive a headless gnoblin-shell via the devkit
# harness, inject Super+Space over mutter RemoteDesktop, and assert the launcher
# spawns (keybind + spawn action + key injection all work) and closes on Escape
# (injected keystrokes reach the client). Needs a dev build in ./install.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-launcher" ] || { echo "SKIP: gnoblin-launcher not built"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== driving Super+Space -> launcher, Escape -> close (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/keybind-launcher-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
exit "$rc"
