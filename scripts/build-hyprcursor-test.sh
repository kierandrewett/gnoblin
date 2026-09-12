#!/usr/bin/env bash
# Build the cursor bridge check against the same Hyprcursor used by Mutter.
set -euo pipefail
cd "$(dirname "$0")/.."
read -r -a cflags <<<"$(pkg-config --cflags glib-2.0 cairo hyprcursor)"
read -r -a libs <<<"$(pkg-config --libs glib-2.0 cairo hyprcursor)"
mkdir -p build
"${CC:-cc}" -Isubprojects/mutter/src "${cflags[@]}" \
    -c subprojects/mutter/src/third_party/xcursor/xcursor.c \
    -o build/hyprcursor-test-xcursor.o
"${CXX:-c++}" -std=c++20 -Ibuild/mutter -Isubprojects/mutter/src "${cflags[@]}" \
    src/cursor/gnoblin-hyprcursor.cpp src/cursor/test-hyprcursor.cpp \
    build/hyprcursor-test-xcursor.o "${libs[@]}" -o build/test-hyprcursor
