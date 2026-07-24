#!/usr/bin/env bash
# Verify that every loose RPM Source declared by the Gnoblin specs is staged.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-rpm-sources-test.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/mutter" "$TMP/gnome-shell"
"$ROOT/scripts/stage-rpm-sources.sh" mutter "$TMP/mutter"
"$ROOT/scripts/stage-rpm-sources.sh" gnome-shell "$TMP/gnome-shell"
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
assert_local_sources gnome-shell "$TMP/gnome-shell"

"$ROOT/scripts/list-tarball-sources.sh" mutter > "$TMP/mutter.sources"
"$ROOT/scripts/list-tarball-sources.sh" gnome-shell > "$TMP/gnome-shell.sources"

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
    done < "$manifest"

    if [ "$found" != true ]; then
        echo "FAIL: $project archive omits required source: $expected" >&2
        exit 1
    fi
}

assert_archive_source mutter subprojects/gvdb/meson.build
assert_archive_source gnome-shell subprojects/gvc/meson.build
assert_archive_source gnome-shell subprojects/libshew/meson.build
assert_archive_source gnome-shell subprojects/jasmine-gjs/meson.build

echo "PASS: RPM sidecars and mandatory archive sources staged"
