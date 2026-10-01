#!/usr/bin/env bash
# Materialise pinned source projects for the CMake/Ninja graph.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "$0")")/.."
ROOT="$(pwd -P)"
mode="${1:?source mode required}"
if [ "$#" -gt 1 ]; then
    projects=("$2")
else
    projects=(mutter xdg-desktop-portal-gnome)
fi
for name in "${projects[@]}"; do
    case "$name" in
        mutter | xdg-desktop-portal-gnome) ;;
        *)
            echo "Unknown source project: $name" >&2
            exit 2
            ;;
    esac
done
if [ "$mode" = release-archive ]; then
    temporary_source=''
    trap 'if [ -n "$temporary_source" ]; then rm -rf -- "$temporary_source"; fi' EXIT
    for name in "${projects[@]}"; do
        destination="subprojects/$name"
        if [ -f "$destination/meson.build" ]; then
            continue
        fi
        if [ -L "$destination" ] || { [ -e "$destination" ] && [ ! -d "$destination" ]; } ||
            { [ -d "$destination" ] && [ -n "$(find "$destination" -mindepth 1 -maxdepth 1 -print -quit)" ]; }; then
            echo "Incomplete release source: $destination. Remove it before retrying." >&2
            exit 1
        fi
        archive_name="$name"
        archives=(sources/"$archive_name"-*.tar.xz)
        if [ "${#archives[@]}" -ne 1 ] || [ ! -f "${archives[0]}" ]; then
            echo "Expected one $archive_name source archive in sources/." >&2
            exit 1
        fi
        mkdir -p subprojects
        temporary_source="$(mktemp -d "subprojects/.$name.extract.XXXXXX")"
        tar -xf "${archives[0]}" -C "$temporary_source" --strip-components=1
        if [ ! -f "$temporary_source/meson.build" ]; then
            echo "Invalid release source: ${archives[0]}" >&2
            exit 1
        fi
        if [ -d "$destination" ]; then
            rmdir -- "$destination"
        fi
        mv -- "$temporary_source" "$destination"
        temporary_source=''
    done
    exit 0
fi
test "$mode" = checkout || {
    echo "Unknown source mode: $mode" >&2
    exit 2
}

for name in "${projects[@]}"; do
    git submodule sync --recursive -- "subprojects/$name"
    submodule_path="$ROOT/subprojects/$name"
    expected_root="$(realpath -m -- "$submodule_path")"
    actual_root="$(git -C "$submodule_path" rev-parse --show-toplevel 2>/dev/null || true)"
    if [ "$actual_root" != "$expected_root" ]; then
        git submodule update --init --recursive -- "subprojects/$name"
    fi
done
./scripts/ensure-release-subprojects.sh "${projects[@]}"
for name in "${projects[@]}"; do
    tag="$(./scripts/gnome-versions.py get "$name" version)"
    ./scripts/subproject-state.sh check "$name" "$tag"
    ./scripts/list-tarball-sources.sh "$name" --prepare >/dev/null
    ./scripts/subproject-state.sh record "$name" "$tag"
done
