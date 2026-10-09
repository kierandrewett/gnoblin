#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
output="${1:-$root/build/frame-renderers}"
mkdir -p "$output"
protocol="$root/src/protocols/window-frame/gnoblin-window-frame-v1.xml"
wayland-scanner client-header "$protocol" "$output/gnoblin-window-frame-v1-client-protocol.h"
wayland-scanner private-code "$protocol" "$output/gnoblin-window-frame-v1-protocol.c"
read -r -a wlflags <<<"$(pkg-config --cflags --libs wayland-client)"
read -r -a cairoflags <<<"$(pkg-config --cflags --libs pangocairo)"
# GNOBLIN_FRAME_RENDERERS selects what to build. The default builds both; "cairo" skips the Qt renderer.
renderers="${GNOBLIN_FRAME_RENDERERS:-cairo qt}"
cc -std=c11 -Wall -Wextra -Wno-unused-parameter -I"$output" -I"$root/src/tools/frame-renderer" -c "$root/src/tools/frame-renderer/client.c" -o "$output/client.o"
cc -I"$output" -c "$output/gnoblin-window-frame-v1-protocol.c" -o "$output/protocol.o"
cc "$output/client.o" "$output/protocol.o" "$root/src/tools/frame-renderer/paint-cairo.c" -o "$output/gnoblin-frame-cairo" "${wlflags[@]}" "${cairoflags[@]}"
case " $renderers " in
    *" qt "*)
        read -r -a qtflags <<<"$(pkg-config --cflags --libs Qt6Gui Qt6Core)"
        c++ -std=c++17 "$output/client.o" "$output/protocol.o" "$root/src/tools/frame-renderer/paint-qt.cpp" -o "$output/gnoblin-frame-qt" "${wlflags[@]}" "${qtflags[@]}"
        ;;
esac
printf 'Built native frame renderers in %s\n' "$output"
