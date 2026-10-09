#!/usr/bin/env bash
# List the tracked and Gnoblin-owned files required by a release source archive.
set -euo pipefail

PROJECT="${1:?usage: list-tarball-sources.sh <mutter|xdg-desktop-portal-gnome> [--prepare]}"
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
    xdg-desktop-portal-gnome)
        REQUIRED_SUBPROJECTS=(libgxdp)
        ;;
    *)
        echo "unknown subproject: $PROJECT" >&2
        exit 1
        ;;
esac

compatibility_patch_for() {
    local dependency="${1:?dependency required}"

    case "$PROJECT:$dependency" in
        xdg-desktop-portal-gnome:libgxdp)
            printf '%s\n' "$ROOT/patches/portal-dependencies/libgxdp/0001-gtk-4.20-compat.patch"
            ;;
        *)
            return 1
            ;;
    esac
}

expected_compatibility_diff() {
    local dependency_root="${1:?dependency root required}"
    local patch="${2:?patch required}"
    local temporary_index

    temporary_index="$(mktemp)"
    rm -f -- "$temporary_index"
    GIT_INDEX_FILE="$temporary_index" git -C "$dependency_root" read-tree HEAD
    GIT_INDEX_FILE="$temporary_index" git -C "$dependency_root" apply --cached "$patch"
    GIT_INDEX_FILE="$temporary_index" git -C "$dependency_root" diff --cached --binary HEAD
    rm -f -- "$temporary_index"
}

compatibility_patch_is_exact() {
    local dependency_root="${1:?dependency root required}"
    local patch="${2:?patch required}"
    local expected actual

    git -C "$dependency_root" diff --cached --quiet || return 1
    expected="$(expected_compatibility_diff "$dependency_root" "$patch")"
    actual="$(git -C "$dependency_root" diff --binary HEAD)"
    [ "$actual" = "$expected" ]
}

apply_compatibility_patch() {
    local dependency="${1:?dependency required}"
    local dependency_root="${2:?dependency root required}"
    local patch

    patch="$(compatibility_patch_for "$dependency" || true)"
    [ -n "$patch" ] || return 0
    [ -f "$patch" ] || {
        echo "missing compatibility patch: $patch" >&2
        exit 1
    }

    if git -C "$dependency_root" diff --quiet &&
        git -C "$dependency_root" diff --cached --quiet; then
        git -C "$dependency_root" apply "$patch"
    elif ! compatibility_patch_is_exact "$dependency_root" "$patch"; then
        echo "required subproject contains unknown local changes: $dependency" >&2
        exit 1
    fi
}

verify_compatibility_patch() {
    local dependency="${1:?dependency required}"
    local dependency_root="${2:?dependency root required}"
    local patch

    patch="$(compatibility_patch_for "$dependency" || true)"
    [ -n "$patch" ] || return 0
    if ! compatibility_patch_is_exact "$dependency_root" "$patch"; then
        echo "required subproject compatibility patch was not applied exactly: $dependency" >&2
        exit 1
    fi
}

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
    done <"$wrap"
}

list_required_subproject() {
    local dependency="${1:?dependency required}"
    local wrap="$SOURCE_ROOT/subprojects/$dependency.wrap"
    local directory revision dependency_root repository_root actual_revision path

    if [ ! -f "$wrap" ]; then
        echo "missing required Meson wrap: $wrap" >&2
        exit 1
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
    if [ "$PREPARE" = true ] && [ "$repository_root" = "$dependency_root" ]; then
        actual_revision="$(git -C "$dependency_root" rev-parse HEAD)"
        if [ "$actual_revision" != "$revision" ]; then
            if ! git -C "$dependency_root" diff --quiet ||
                ! git -C "$dependency_root" diff --cached --quiet; then
                echo "required subproject contains tracked changes: $dependency" >&2
                exit 1
            fi
            meson subprojects update --reset \
                --sourcedir "$SOURCE_ROOT" "$dependency" >&2
        fi
    elif [ "$PREPARE" = true ]; then
        # GNOME's GitLab sometimes answers 503. meson reports the failed fetch as a warning and still exits 0, so the
        # exit status cannot decide whether to retry. Retry until the subproject is a Git checkout with a commit.
        #
        # meson also treats any existing directory as already downloaded. So a half-made directory from a failed
        # attempt has to go before the next one, but only a directory that this run created. A directory that
        # someone else filled is not ours to delete, and an empty one would stop meson from fetching at all.
        if [ -e "$dependency_root" ]; then
            if [ -n "$(find "$dependency_root" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]; then
                echo "subproject $dependency exists but is not a Git checkout: $dependency_root" >&2
                echo "Move it away or remove it, then run: $0 $PROJECT --prepare" >&2
                exit 1
            fi
            rmdir -- "$dependency_root"
        fi
        attempt=1
        while true; do
            meson subprojects download --sourcedir "$SOURCE_ROOT" "$dependency" >&2 || true
            if [ "$(git -C "$dependency_root" rev-parse --show-toplevel 2>/dev/null || true)" = "$dependency_root" ] &&
                git -C "$dependency_root" rev-parse --verify --quiet HEAD >/dev/null; then
                break
            fi
            if [ "$attempt" -ge 4 ]; then
                echo "could not download subproject $dependency after $attempt attempts" >&2
                exit 1
            fi
            echo "[tarball] download of $dependency failed (attempt $attempt of 4); retrying" >&2
            # This run made the directory (it was absent or empty above), so removing it is safe.
            rm -rf -- "${dependency_root:?}"
            sleep $((attempt * 10))
            attempt=$((attempt + 1))
        done
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

    if [ "$PREPARE" = true ]; then
        apply_compatibility_patch "$dependency" "$dependency_root"
    fi
    verify_compatibility_patch "$dependency" "$dependency_root"

    if ! git -C "$dependency_root" diff --quiet || ! git -C "$dependency_root" diff --cached --quiet; then
        patch="$(compatibility_patch_for "$dependency" || true)"
        if [ -z "$patch" ]; then
            echo "required subproject contains tracked changes: $dependency" >&2
            exit 1
        fi
        # verify_compatibility_patch above has proved the exact known diff.
    fi

    while IFS= read -r -d '' path; do
        printf 'subprojects/%s/%s\0' "$directory" "$path"
    done < <(git -C "$dependency_root" ls-files --cached -z)
}

git -C "$SOURCE_ROOT" ls-files --cached -z
for dependency in "${REQUIRED_SUBPROJECTS[@]}"; do
    list_required_subproject "$dependency"
done
cmake -DACTION=overlay "-DPROJECT=$PROJECT" "-DSOURCE_DIR=$SOURCE_ROOT" -DMODE=list -P "$ROOT/cmake/source-step.cmake" |
    tr '\n' '\0'
