#!/usr/bin/env bash
# Verify that every loose RPM Source declared by the Gnoblin specs is staged.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-rpm-sources-test.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/mutter"
"$ROOT/scripts/stage-rpm-sources.sh" mutter "$TMP/mutter"
assert_local_sources() {
    local project="${1:?project required}"
    local output_dir="${2:?output directory required}"
    local declaration source

    while IFS= read -r declaration; do
        source="${declaration#*:}"
        source="${source#"${source%%[![:space:]]*}"}"
        source="${source##*/}"
        [ -f "$output_dir/$source" ] || {
            echo "FAIL: $project spec source was not staged: $source" >&2
            exit 1
        }
    done < <(grep -E '^Source[1-9][0-9]*:' "$ROOT/packaging/rpm/$project.spec")
}

assert_local_sources mutter "$TMP/mutter"

"$ROOT/scripts/list-tarball-sources.sh" mutter >"$TMP/mutter.sources"
"$ROOT/scripts/list-tarball-sources.sh" xdg-desktop-portal-gnome >"$TMP/xdg-desktop-portal-gnome.sources"

mkdir -p "$TMP/mutter-tarball"
mutter_version="$("$ROOT/scripts/gnome-versions.py" get mutter version)"
"$ROOT/scripts/make-tarball.sh" mutter "$TMP/mutter-tarball" >/dev/null
tar -tf "$TMP/mutter-tarball/mutter-$mutter_version.tar.xz" >"$TMP/mutter-tarball.contents"
grep -Fx "mutter-$mutter_version/src/wayland/protocol/wlr-layer-shell-unstable-v1.xml" \
    "$TMP/mutter-tarball.contents" >/dev/null
grep -Fx "mutter-$mutter_version/src/wayland/meta-wayland-layer-shell.c" \
    "$TMP/mutter-tarball.contents" >/dev/null

assert_archive_source() {
    local project="${1:?project required}"
    local expected="${2:?expected source path required}"
    local manifest="$TMP/$project.sources"
    local source found=false

    while IFS= read -r -d '' source; do
        if [ "$source" = "$expected" ]; then
            found=true
            break
        fi
    done <"$manifest"

    if [ "$found" != true ]; then
        echo "FAIL: $project archive omits required source: $expected" >&2
        exit 1
    fi
}

assert_archive_source mutter subprojects/gvdb/meson.build
assert_archive_source xdg-desktop-portal-gnome subprojects/libgxdp/meson.build

# Release archives unpack upstream sources without their Git metadata. Overlay
# copying must work there as well as in a checked-out submodule.
mkdir -p "$TMP/mutter-archive"
cmake -DACTION=overlay -DPROJECT=mutter "-DSOURCE_DIR=$TMP/mutter-archive" -DMODE=copy \
    -P "$ROOT/cmake/source-step.cmake" >/dev/null
cmp "$ROOT/src/native-control/gnoblin-native-control.c" \
    "$TMP/mutter-archive/src/core/gnoblin-native-control.c"

# In a real submodule, the copied overlay remains invisible to Git status.
mkdir -p "$TMP/mutter-checkout"
git -C "$TMP/mutter-checkout" init -q
cmake -DACTION=overlay -DPROJECT=mutter "-DSOURCE_DIR=$TMP/mutter-checkout" -DMODE=copy \
    -P "$ROOT/cmake/source-step.cmake" >/dev/null
cmp "$ROOT/src/native-control/gnoblin-native-control.c" \
    "$TMP/mutter-checkout/src/core/gnoblin-native-control.c"
test -z "$(git -C "$TMP/mutter-checkout" status --porcelain)"

echo "PASS: RPM sidecars and mandatory archive sources staged"
