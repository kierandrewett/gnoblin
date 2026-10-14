#!/usr/bin/env bash
# Builds the WiVRn streaming server from the pinned subprojects/wivrn checkout. gnoblin-vr runs it
# for standalone headsets (Quest, Pico, Vive Focus) over USB or Wi-Fi. Distributions that package
# WiVRn do not need this: install their wivrn package instead.
#
#   scripts/build-vr-streamer.sh            build into build/wivrn
#   scripts/build-vr-streamer.sh install    build, then install to GNOBLIN_VR_STREAMER_PREFIX
#
# WiVRn fetches its Monado fork while configuring, so the first build needs network access.
# Extra CMake options go in WIVRN_CMAKE_ARGS, for example WIVRN_CMAKE_ARGS=-DWIVRN_USE_X264=OFF on a
# distribution without x264.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build="${GNOBLIN_VR_STREAMER_BUILD:-$root/build/wivrn}"
prefix="${GNOBLIN_VR_STREAMER_PREFIX:-/usr/local}"

git -C "$root" submodule update --init --depth 1 subprojects/wivrn

# WiVRn stamps its version from git, which a submodule checkout without tags cannot answer.
wivrn_src="$root/subprojects/wivrn"
describe="$(git -C "$wivrn_src" describe --tags --always 2>/dev/null || true)"
commit="$(git -C "$wivrn_src" rev-parse HEAD)"

cmake -S "$wivrn_src" -B "$build" -GNinja \
    -DGIT_DESC="${describe:-$commit}" \
    -DGIT_COMMIT="$commit" \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DWIVRN_BUILD_CLIENT=OFF \
    -DWIVRN_BUILD_DASHBOARD=OFF \
    -DWIVRN_BUILD_SERVER=ON \
    ${WIVRN_CMAKE_ARGS:-}
cmake --build "$build" --parallel "${JOBS:-$(nproc)}"

if [ "${1:-}" = install ]; then
    sudo cmake --install "$build"
    echo "Installed wivrn-server to $prefix. gnoblin-vr finds it through PATH."
fi
