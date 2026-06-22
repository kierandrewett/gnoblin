#!/usr/bin/env bash
# Regression: late virtual monitor materialization followed by repeated size
# renegotiation must not crash the Slint layer-shell clients or compositor.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0"; exit 0; }
command -v gst-inspect-1.0 >/dev/null || { echo "SKIP: no gst-inspect-1.0"; exit 0; }
gst-inspect-1.0 pipewiresrc >/dev/null 2>&1 || { echo "SKIP: no GStreamer pipewiresrc plugin"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
[ -S "$runtime/pipewire-0" ] || [ -S "$runtime/pipewire-0-manager" ] || {
  echo "SKIP: no PipeWire socket in $runtime"
  exit 0
}

echo "== late-monitor configure storm must not kill layer-shell clients =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/scripts/devkit-harness.py" storm 2>&1 \
  | grep -vE "vfs|fusermount|gvfs|can't be made"
rc=${PIPESTATUS[0]}
if [ "$rc" -eq 2 ]; then
  echo "SKIP: virtual monitor never materialized; configure storm was inconclusive"
  exit 0
fi
exit "$rc"
