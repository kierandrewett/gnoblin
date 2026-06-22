#!/usr/bin/env bash
# Regression: the [output] -> org.gnome.Mutter.DisplayConfig bridge. Boots a
# headless gnoblin-shell with `[output] Meta-0 = transform 90` and asserts the
# virtual monitor was rotated (transform=1 in GetCurrentState) via an async
# ApplyMonitorsConfig that runs exactly once (no reconfigure loop). Needs a dev
# build; no grim/foot needed.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== [output] config -> DisplayConfig bridge (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/output-config-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
exit "${PIPESTATUS[0]}"
