#!/usr/bin/env bash
# Regression: shell-ui full-screen clients must update app-level geometry after a
# post-map monitor resize, not only resize the underlying Slint window.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-power-menu" ] || { echo "SKIP: no gnoblin-power-menu at $PREFIX (run 'just dev')"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0"; exit 0; }
command -v gst-inspect-1.0 >/dev/null || { echo "SKIP: no gst-inspect-1.0"; exit 0; }
gst-inspect-1.0 pipewiresrc >/dev/null 2>&1 || { echo "SKIP: no GStreamer pipewiresrc plugin"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
[ -S "$runtime/pipewire-0" ] || [ -S "$runtime/pipewire-0-manager" ] || {
  echo "SKIP: no PipeWire socket in $runtime"
  exit 0
}

echo "== power menu must recenter after monitor resize (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/power-menu-resize-test.py" 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
