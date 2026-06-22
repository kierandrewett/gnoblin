#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

if [ ! -x "$PREFIX/bin/gnoblin-shell" ] || [ ! -x "$PREFIX/bin/gnoblin-night-light" ]; then
  echo "SKIP: gnoblin-shell/gnoblin-night-light not installed in $PREFIX (run just dev)" >&2
  exit 0
fi

echo "== Night Light must bind gamma control for hotplugged outputs =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/night-light-hotplug-test.py" 2>&1 \
  | tee /tmp/gnoblin-night-light-hotplug-test.log
