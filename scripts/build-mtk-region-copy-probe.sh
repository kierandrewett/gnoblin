#!/usr/bin/env bash
# Build a preload probe for real MtkRegion-copy counts in a private compositor.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="${1:?usage: build-mtk-region-copy-probe.sh OUTPUT_DIRECTORY}"

mkdir -p "$OUTPUT"
cc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
   "$ROOT/tests/mtk-region-copy-probe.c" \
   -ldl -pthread \
   -o "$OUTPUT/libmtk-region-copy-probe.so"
