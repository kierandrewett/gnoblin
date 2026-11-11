#!/usr/bin/env bash
# Build wlcs (Canonical's Wayland Conformance Suite) and the Gnoblin integration library, run the core subset
# against the Rust compositor and check the result against tests/wlcs/expected.toml.
#
#   scripts/wlcs.sh                       build both, run the core subset, check it, print pass/fail/skip counts
#   scripts/wlcs.sh --gate                run only the tests expected.toml lists as passing (what CI runs)
#   scripts/wlcs.sh --all                 run every wlcs test, not just the core subset
#   scripts/wlcs.sh --update              rewrite the `passing` list of expected.toml from this run
#   scripts/wlcs.sh --build               build only
#   scripts/wlcs.sh 'XdgToplevelStableTest.*'
#                                         run the matching tests and print each one's output
#
# wlcs is GPL-2.0. It is cloned into target/wlcs and built there, never vendored, linked or shipped.
#
# Needs: git, cmake, a C++20 compiler, pkg-config, the wayland development files (libwayland-dev) and
# GoogleTest with GoogleMock (libgtest-dev libgmock-dev), Boost headers (libboost-dev) and python3 >= 3.11.
#   Debian/Ubuntu: apt-get install git cmake g++ pkg-config libwayland-dev libgtest-dev libgmock-dev libboost-dev python3
#   Fedora:        dnf install git cmake gcc-c++ pkgconf wayland-devel gtest-devel gmock-devel boost-devel python3
#
# WLCS_REF pins the wlcs release; raising it is a deliberate change that needs the expectation file re-checked.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

WLCS_REF="${WLCS_REF:-v1.8.1}"
WLCS_REPO="${WLCS_REPO:-https://github.com/canonical/wlcs}"
target="${CARGO_TARGET_DIR:-$root/target}"
wlcs_src="${WLCS_SRC:-$target/wlcs/src}"
wlcs_build="${WLCS_BUILD:-$target/wlcs/build}"
profile="${WLCS_PROFILE:-debug}"

mode=run
subset=core
runner_args=()
patterns=()
while (($#)); do
    case "$1" in
        --build) mode=build ;;
        --gate) runner_args+=(--only-passing) ;;
        --all) subset=all ;;
        --update) runner_args+=(--update) ;;
        --jobs)
            shift
            runner_args+=(--jobs "$1")
            ;;
        -h | --help)
            sed -n 2,23p "$0"
            exit 0
            ;;
        -*)
            echo "wlcs.sh: unknown option $1" >&2
            exit 2
            ;;
        *) patterns+=("$1") ;;
    esac
    shift
done

need() {
    command -v "$1" >/dev/null || {
        echo "wlcs.sh: $1 is required; see the header of this script for the packages" >&2
        exit 2
    }
}
for tool in git cmake pkg-config python3 cargo; do need "$tool"; done
for module in wayland-client wayland-server wayland-scanner; do
    pkg-config --exists "$module" || {
        echo "wlcs.sh: pkg-config cannot find $module (libwayland-dev)" >&2
        exit 2
    }
done

build_wlcs() {
    if [ ! -d "$wlcs_src/.git" ]; then
        mkdir -p "$(dirname "$wlcs_src")"
        git clone --quiet --depth 1 --branch "$WLCS_REF" "$WLCS_REPO" "$wlcs_src"
    fi
    git -C "$wlcs_src" fetch --quiet --depth 1 origin "refs/tags/$WLCS_REF:refs/tags/$WLCS_REF" 2>/dev/null || true
    git -C "$wlcs_src" checkout --quiet --force "$WLCS_REF"
    # Two fixes to wlcs's own build files, applied to the checkout and nothing else:
    #  - it compiles with -Werror, and GCC 12's -Wrestrict false positive in libstdc++ fails the build;
    #  - it passes `-DNAME=value` through COMPILE_DEFINITIONS, which CMake before 3.26 turns into `-D-DNAME=value`.
    sed -i 's/ -Werror"/"/' "$wlcs_src/CMakeLists.txt"
    sed -i 's/COMPILE_DEFINITIONS -D/COMPILE_DEFINITIONS /' "$wlcs_src/CMakeLists.txt"
    # And the patches in tests/wlcs/patches, each a fix for a bug in the suite itself (see the patch headers).
    for patch in "$root"/tests/wlcs/patches/*.patch; do
        git -C "$wlcs_src" apply "$patch"
    done
    # wlcs compiles with warnings that are not ours to fix, so the output is kept in a log and shown only on failure.
    mkdir -p "$wlcs_build"
    local log="$wlcs_build/build.log"
    {
        cmake -S "$wlcs_src" -B "$wlcs_build" -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_STANDARD=20 \
            -DCMAKE_CXX_FLAGS=-Wno-error -Wno-dev &&
            cmake --build "$wlcs_build" --target wlcs --parallel "$(nproc)"
    } >"$log" 2>&1 || {
        tail -40 "$log" >&2
        echo "wlcs.sh: building wlcs failed; the full log is $log" >&2
        exit 1
    }
}

build_plugin() {
    local flags=()
    [ "$profile" = release ] && flags+=(--release)
    cargo build --locked -p gnoblin-wlcs "${flags[@]}"
}

build_wlcs
build_plugin
[ "$mode" = build ] && exit 0

# Every test gets its own socket in a private directory, but libwayland-client wants a runtime directory to exist.
if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
    XDG_RUNTIME_DIR="$(mktemp -d)"
    export XDG_RUNTIME_DIR
    trap 'rm -rf "$XDG_RUNTIME_DIR"' EXIT
fi

exec python3 "$root/scripts/wlcs-run.py" \
    --wlcs "$wlcs_build/wlcs" \
    --plugin "$target/$profile/libgnoblin_wlcs.so" \
    --subset "$subset" \
    --results "$target/wlcs/results.json" \
    "${runner_args[@]}" "${patterns[@]}"
