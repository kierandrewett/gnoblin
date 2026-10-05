#!/usr/bin/env bash
# Copy gnoblin's overlay source files into a submodule.
#
# Large new files we author (e.g. the layer-shell implementation) live as real,
# tracked source under src/<feature>/ and are copied verbatim into the submodule
# here — they are NOT shipped as additive patches. Each src/<feature>/manifest
# maps a source file to its destination path inside a subproject. In a Git
# submodule, copied files are added to .git/info/exclude and removed by
# `git clean` on reset. Release archives have no Git metadata, so copying there
# does not need an exclude entry.
#
# Usage: copy-overlay.sh <project> <submodule-dir> [--list-destinations|--remove-destinations]
set -euo pipefail

PROJ="${1:?usage: copy-overlay.sh <project> <submodule-dir>}"
SM="${2:?usage: copy-overlay.sh <project> <submodule-dir>}"
MODE="${3:-copy}"
case "$MODE" in
    copy | --list-destinations | --remove-destinations) ;;
    *)
        echo "unknown overlay action: $MODE" >&2
        exit 1
        ;;
esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
case "$PROJ" in
    mutter | xdg-desktop-portal-gnome) ;;
    *)
        echo "unsupported overlay project: $PROJ" >&2
        exit 2
        ;;
esac

n=0
# Manifests live under src/ at any depth (src/protocols/<feature>/manifest,
# src/config/manifest, …) so the source tree can be reorganised by role freely.
while IFS= read -r manifest; do
    feature_dir="$(dirname "$manifest")"
    while read -r proj src dest _rest; do
        [[ -z "${proj:-}" || "$proj" == \#* ]] && continue
        [[ "$proj" != "$PROJ" ]] && continue
        [[ -f "$feature_dir/$src" ]] || {
            echo "overlay: missing $feature_dir/$src" >&2
            exit 1
        }
        case "$dest" in
            "" | .. | /* | ../* | */../* | */..)
                echo "overlay: invalid destination outside subproject: $dest" >&2
                exit 1
                ;;
        esac
        if [[ "$MODE" == --list-destinations ]]; then
            printf '%s\0' "$dest"
            continue
        fi
        if [[ "$MODE" == --remove-destinations ]]; then
            rm -f -- "$SM/$dest"
            rmdir --ignore-fail-on-non-empty "$SM/$(dirname "$dest")" 2>/dev/null || true
            n=$((n + 1))
            continue
        fi
        mkdir -p "$SM/$(dirname "$dest")"
        cp "$feature_dir/$src" "$SM/$dest"
        # Keep a submodule's Git status clean; source archives have no Git
        # metadata and do not need a local exclude file.
        if git -C "$SM" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            git_dir="$(git -C "$SM" rev-parse --absolute-git-dir)"
            excl="$git_dir/info/exclude"
            [ -f "$excl" ] && ! grep -qxF "/$dest" "$excl" 2>/dev/null && echo "/$dest" >>"$excl"
        fi
        n=$((n + 1))
    done <"$manifest"
done < <(find "$ROOT/src" -name manifest -type f | sort)
if [[ "$MODE" == copy ]]; then
    echo ">> copied $n overlay file(s) into $PROJ"
elif [[ "$MODE" == --remove-destinations ]]; then
    echo ">> removed $n overlay file(s) from $PROJ"
fi
