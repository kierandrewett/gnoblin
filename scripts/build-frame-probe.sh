#!/usr/bin/env bash
# Build the test-only framebuffer reader against this checkout's compositor.
set -euo pipefail
task_root="$(cd "$(dirname "$0")/.." && pwd)"
probe_output="${1:?usage: build-frame-probe.sh OUTPUT_DIRECTORY}"
mkdir -p "$probe_output"
export PKG_CONFIG_PATH="${GNOBLIN_PREFIX:-$task_root/install}/lib64/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
read -r -a probe_flags <<<"$(pkg-config --cflags --libs mutter-clutter-17 gdk-pixbuf-2.0)"
probe_namespace="${FRAME_PROBE_NAMESPACE:-FrameProbe}"
probe_library="frame-probe-$probe_namespace"
gcc -shared -fPIC "$task_root/tests/frame-probe/probe.c" -o "$probe_output/lib$probe_library.so" "${probe_flags[@]}" -L"${GNOBLIN_PREFIX:-$task_root/install}/lib64" -lmutter-17
LD_LIBRARY_PATH="${GNOBLIN_PREFIX:-$task_root/install}/lib64:${GNOBLIN_PREFIX:-$task_root/install}/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    g-ir-scanner --quiet --namespace="$probe_namespace" --nsversion=1.0 \
    --identifier-prefix=FrameProbe --symbol-prefix=frame_probe \
    --library="$probe_library" --library-path="$probe_output" \
    --include-uninstalled="$task_root/build/mutter/clutter/clutter/Clutter-17.gir" \
    --add-include-path="$task_root/build/mutter/cogl/cogl" --add-include-path="$task_root/build/mutter/mtk/mtk" \
    --pkg=mutter-clutter-17 --pkg=gdk-pixbuf-2.0 \
    "$task_root/tests/frame-probe/probe.h" "$task_root/tests/frame-probe/probe.c" -o "$probe_output/$probe_namespace-1.0.gir"
g-ir-compiler --includedir="$task_root/build/mutter/clutter/clutter" \
    --includedir="$task_root/build/mutter/cogl/cogl" --includedir="$task_root/build/mutter/mtk/mtk" \
    "$probe_output/$probe_namespace-1.0.gir" -o "$probe_output/$probe_namespace-1.0.typelib"
