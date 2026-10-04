#!/usr/bin/env bash
# Build and run the config/runtime tests through the production CMake graph.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
jobs="${GNOBLIN_BUILD_JOBS:-4}"
mkdir -p "$ROOT/build/tmp"
build="$(mktemp -d "$ROOT/build/tmp/config-tests.XXXXXX")"
trap 'rm -rf -- "$build"' EXIT

cmake -S "$ROOT" -B "$build" -G Ninja \
    -DGNOBLIN_PREFIX="$build/install" \
    -DGNOBLIN_BUILD_TYPE=debugoptimized
cmake --build "$build" \
    --target lua-config-test lua-api-test glob-config-test \
    --parallel "$jobs"
mkdir -p "$build/install/share/gnoblin"
cat >"$build/install/share/gnoblin/native-keybindings.json" <<'JSON'
{"format":1,"groups":{"wm":["close","lower"],"mutter":[],"wayland":[]}}
JSON
GNOBLIN_PREFIX="$build/install" ctest --test-dir "$build" --output-on-failure \
    -R '^(lua-config|lua-api|glob-config)$'
