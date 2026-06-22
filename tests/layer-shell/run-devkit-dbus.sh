#!/usr/bin/env bash
# Regression: the isolated devkit DBus helper must activate the Documents stub
# even when the generated service paths contain spaces.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

command -v dbus-run-session >/dev/null || { echo "SKIP: no dbus-run-session"; exit 0; }
python3 -c "import gi" 2>/dev/null || { echo "SKIP: needs python gi"; exit 0; }

echo "== devkit DBus helper must activate Documents stub (spaced paths) =="
python3 "$ROOT/tests/layer-shell/devkit-dbus-test.py"
