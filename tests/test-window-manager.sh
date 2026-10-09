#!/usr/bin/env bash
# Mutter's own unit, Wayland, backend and focus suites, built from the patched subproject.
# Run this on a real host before a release. It patches subprojects/mutter and builds build/mutter-tests.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

prefix="${GNOBLIN_PREFIX:-$PWD/install}"
buildtype="${GNOBLIN_BUILD_TYPE:-debugoptimized}"
options=(
    "--prefix=$prefix" --libdir=lib64 "--buildtype=$buildtype"
    -Ddevkit=enabled -Dtests=enabled -Dmutter_tests=true -Dclutter_tests=false -Dcogl_tests=false
    -Ddocs=false -Dprofiler=false "-Dudev_dir=$prefix/lib/udev"
)
suites=(--suite mutter:mutter/unit --suite mutter:mutter/wayland --suite mutter:mutter/backends/native)
focus_tests=(
    mutter:focus-default-window-globally-active-input mutter:click-to-focus-and-raise mutter:overview-focus
    mutter:sloppy-focus mutter:sloppy-focus-pointer-rest mutter:sloppy-focus-auto-raise mutter:popup-focus
)

run() {
    printf '+ %s\n' "$*"
    "$@"
}

# Count the tests a selection names, and stop if it names none, so an empty selection cannot pass.
require_tests() {
    local label="$1" count
    shift
    count="$(meson test -C build/mutter-tests "$@" --list | sed '/^$/d' | wc -l)"
    if [ "$count" -le 0 ]; then
        echo "FAIL: no Mutter $label tests selected" >&2
        exit 1
    fi
    echo ">> running $count Mutter $label tests"
}

run ./scripts/apply-patches.sh mutter
run meson setup --reconfigure build/mutter-tests subprojects/mutter "${options[@]}" ||
    run meson setup build/mutter-tests subprojects/mutter "${options[@]}"
run meson compile -C build/mutter-tests

require_tests "unit, Wayland and native" "${suites[@]}"
run meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs "${suites[@]}"

require_tests "focus and stacking" "${focus_tests[@]}"
run meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs "${focus_tests[@]}"
