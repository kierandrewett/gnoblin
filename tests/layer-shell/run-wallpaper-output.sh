#!/usr/bin/env bash
# Regression: gnoblin-wallpaper honors explicit --output binding. Boots two
# virtual outputs, starts one wallpaper client per output with distinct images,
# and pixel-checks that each monitor shows the correct wallpaper.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-wallpaper" ] || { echo "SKIP: gnoblin-wallpaper not built"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
python3 -c "import gi, PIL" 2>/dev/null || { echo "SKIP: needs python gi + PIL"; exit 0; }

echo "== wallpaper --output must bind independently on two monitors (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/wallpaper-output-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
