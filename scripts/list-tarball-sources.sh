#!/usr/bin/env bash
# List the tracked and Gnoblin-owned files required by a release source archive.
set -euo pipefail

PROJECT="${1:?usage: list-tarball-sources.sh <mutter|gnome-shell> [--prepare]}"
PREPARE=false
if [ "${2:-}" = "--prepare" ]; then
    PREPARE=true
elif [ -n "${2:-}" ]; then
    echo "unknown option: $2" >&2
    exit 1
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_ROOT="$ROOT/subprojects/$PROJECT"

case "$PROJECT" in
    mutter)
        REQUIRED_SUBPROJECTS=(gvdb)
        ;;
    gnome-shell)
        REQUIRED_SUBPROJECTS=(gvc libshew jasmine-gjs)
        ;;
    *)
        echo "unknown subproject: $PROJECT" >&2
        exit 1
        ;;
esac

read_wrap_git_value() {
    local wrap="${1:?wrap required}"
    local wanted="${2:?key required}"
    local line key value in_wrap=false

    while IFS= read -r line; do
        case "$line" in
            "[wrap-git]")
                in_wrap=true
                continue
                ;;
            "["*"]")
                in_wrap=false
                continue
                ;;
        esac

        if [ "$in_wrap" != true ] || [[ "$line" != *=* ]]; then
            continue
        fi

        key="${line%%=*}"
        key="${key#"${key%%[![:space:]]*}"}"
        key="${key%"${key##*[![:space:]]}"}"
        if [ "$key" != "$wanted" ]; then
            continue
        fi

        value="${line#*=}"
        value="${value#"${value%%[![:space:]]*}"}"
        value="${value%"${value##*[![:space:]]}"}"
        printf '%s\n' "$value"
        return
    done < "$wrap"
}

list_required_subproject() {
    local dependency="${1:?dependency required}"
    local wrap="$SOURCE_ROOT/subprojects/$dependency.wrap"
    local directory revision dependency_root repository_root actual_revision path

    if [ ! -f "$wrap" ]; then
        echo "missing required Meson wrap: $wrap" >&2
        exit 1
    fi

    if [ "$PREPARE" = true ]; then
        meson subprojects download --sourcedir "$SOURCE_ROOT" "$dependency" >&2
    fi

    directory="$(read_wrap_git_value "$wrap" directory || true)"
    revision="$(read_wrap_git_value "$wrap" revision || true)"
    directory="${directory:-$dependency}"
    dependency_root="$SOURCE_ROOT/subprojects/$directory"

    if [ -z "$revision" ]; then
        echo "required subproject is not a pinned Git wrap: $dependency" >&2
        exit 1
    fi

    repository_root="$(git -C "$dependency_root" rev-parse --show-toplevel 2>/dev/null || true)"
    if [ "$repository_root" != "$dependency_root" ]; then
        echo "required subproject is not materialised: $dependency" >&2
        echo "run: $0 $PROJECT --prepare" >&2
        exit 1
    fi

    actual_revision="$(git -C "$dependency_root" rev-parse HEAD)"
    if [ "$actual_revision" != "$revision" ]; then
        echo "required subproject $dependency is at $actual_revision, expected $revision" >&2
        exit 1
    fi

    if ! git -C "$dependency_root" diff --quiet || ! git -C "$dependency_root" diff --cached --quiet; then
        echo "required subproject contains tracked changes: $dependency" >&2
        exit 1
    fi

    while IFS= read -r -d '' path; do
        printf 'subprojects/%s/%s\0' "$directory" "$path"
    done < <(git -C "$dependency_root" ls-files --cached -z)
}

git -C "$SOURCE_ROOT" ls-files --cached -z
for dependency in "${REQUIRED_SUBPROJECTS[@]}"; do
    list_required_subproject "$dependency"
done
"$ROOT/scripts/copy-overlay.sh" "$PROJECT" "$SOURCE_ROOT" --list-destinations
