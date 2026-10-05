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
prepare_gsettings_desktop_schemas() {
    local mode="${1:?source mode required}"
    local version revision source_root archive extracted actual
    version="$(./scripts/gnome-versions.py get gsettings-desktop-schemas version)"
    revision="$(./scripts/gnome-versions.py get gsettings-desktop-schemas commit)"
    source_root="$ROOT/build/source-inputs/gsettings-desktop-schemas-pinned"

    if [ "$mode" = release-archive ]; then
        archive="$ROOT/sources/gsettings-desktop-schemas-$version.tar.xz"
        if [ -d "$source_root" ]; then
            if [ "$(cat "$source_root/GNOBLIN_SOURCE_REVISION" 2>/dev/null || true)" = "$revision" ] &&
                [ -f "$source_root/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in" ]; then
                return
            fi
            if [ "$(git -C "$source_root" rev-parse HEAD 2>/dev/null || true)" = "$revision" ] &&
                [ -z "$(git -C "$source_root" status --porcelain 2>/dev/null || true)" ]; then
                return
            fi
            echo "refusing to replace existing schema source without the pinned revision marker: $source_root" >&2
            exit 1
        fi
        [ -f "$archive" ] || {
            echo "missing pinned schema source archive: $archive" >&2
            exit 1
        }
        mkdir -p "$(dirname "$source_root")"
        extracted="$(mktemp -d "$ROOT/build/source-inputs/.gsettings-desktop-schemas.XXXXXX")"
        tar -xf "$archive" -C "$extracted" --strip-components=1
        actual="$(cat "$extracted/GNOBLIN_SOURCE_REVISION" 2>/dev/null || true)"
        if [ "$actual" != "$revision" ]; then
            rm -rf -- "$extracted"
            echo "schema source archive revision is '$actual', expected '$revision'" >&2
            exit 1
        fi
        [ -f "$extracted/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in" ] || {
            rm -rf -- "$extracted"
            echo "schema source archive is missing the pinned WM schema" >&2
            exit 1
        }
        mv -- "$extracted" "$source_root"
        return
    fi

    [ "$mode" = checkout ] || {
        echo "unknown source mode: $mode" >&2
        exit 2
    }
    if [ -d "$source_root" ] &&
        [ "$(cat "$source_root/GNOBLIN_SOURCE_REVISION" 2>/dev/null || true)" = "$revision" ] &&
        [ -f "$source_root/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in" ]; then
        return
    fi
    if [ -e "$source_root" ] && ! git -C "$source_root" rev-parse --git-dir >/dev/null 2>&1; then
        echo "schema source cache is not a Git checkout: $source_root" >&2
        exit 1
    fi
    if ! git -C "$source_root" rev-parse --git-dir >/dev/null 2>&1; then
        mkdir -p "$(dirname "$source_root")"
        git clone --quiet --filter=blob:none \
            https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas.git "$source_root"
    fi
    if [ -n "$(git -C "$source_root" status --porcelain)" ]; then
        echo "pinned schema source cache contains local changes: $source_root" >&2
        exit 1
    fi
    if [ "$(git -C "$source_root" rev-parse HEAD 2>/dev/null || true)" != "$revision" ]; then
        git -C "$source_root" fetch --quiet --depth=1 origin "$revision"
        git -C "$source_root" checkout --quiet --detach "$revision"
    fi
    actual="$(git -C "$source_root" rev-parse HEAD)"
    if [ "$actual" != "$revision" ]; then
        echo "schema source checkout is at $actual, expected $revision" >&2
        exit 1
    fi
}

if [ "${projects[0]}" = gsettings-desktop-schemas ]; then
    [ "${#projects[@]}" = 1 ] || {
        echo "gsettings-desktop-schemas must be prepared on its own" >&2
        exit 2
    }
    prepare_gsettings_desktop_schemas "$mode"
    exit 0
fi
for name in "${projects[@]}"; do
    case "$name" in
        mutter | xdg-desktop-portal-gnome | gsettings-desktop-schemas) ;;
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
