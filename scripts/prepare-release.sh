#!/usr/bin/env bash
# Prepare a Gnoblin release: set the version, commit and tag.
#
# usage: prepare-release.sh [--dry-run] [--no-tag] [--sign] VERSION
#
# VERSION is a plain SemVer number such as 0.1.11, without the gnoblin-v prefix. The script edits
# src/gnoblin-version.json, which is the one place that holds the version (pkgsmith/ and the source bundle read it),
# commits that file and creates the gnoblin-vVERSION tag on the new commit. It never pushes. It prints the push
# commands, so you push the commit first, wait for the dry run of the release workflow on main, and push the tag last.
set -euo pipefail
export PATH="$PATH"

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

dry_run=false
make_tag=true
sign=false
version=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) dry_run=true ;;
        --no-tag) make_tag=false ;;
        --sign) sign=true ;;
        -*)
            echo "usage: $0 [--dry-run] [--no-tag] [--sign] VERSION" >&2
            exit 2
            ;;
        *)
            [ -z "$version" ] || {
                echo "Give one version only." >&2
                exit 2
            }
            version="$1"
            ;;
    esac
    shift
done
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
    echo "[release] VERSION must look like 1.2.3, got: ${version:-nothing}" >&2
    exit 2
}
tag="gnoblin-v$version"

current="$(python3 scripts/gnoblin-version.py get version)"
if [ "$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n 1)" != "$version" ] || [ "$current" = "$version" ]; then
    echo "[release] $version is not newer than the current version $current" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "[release] tag $tag already exists" >&2
    exit 1
fi
# Submodule pins are not part of a release commit. Anything else uncommitted would be left out of the tag silently.
if [ -n "$(git status --porcelain --ignore-submodules=all)" ]; then
    echo "[release] the working tree has uncommitted changes:" >&2
    git status --short --ignore-submodules=all >&2
    exit 1
fi
branch="$(git branch --show-current)"
if [ "$branch" != main ]; then
    echo "[release] warning: releasing from '$branch', not main" >&2
fi

echo "[release] $current -> $version ($tag)"
if [ "$dry_run" = true ]; then
    echo "[release] dry run: nothing changed"
    exit 0
fi

python3 - "$version" <<'PY'
import json
import sys
from pathlib import Path

path = Path("src/gnoblin-version.json")
data = json.loads(path.read_text())
data["version"] = sys.argv[1]
path.write_text(json.dumps(data, indent=2) + "\n")
PY

mapfile -t changed < <(git diff --name-only --ignore-submodules=all)
if [ "${#changed[@]}" -eq 0 ]; then
    echo "[release] the version change produced no file changes" >&2
    exit 1
fi
printf '[release] changed: %s\n' "${changed[@]}"
git add -- "${changed[@]}"
git commit -m "chore(release): prepare Gnoblin $version" \
    -m "Sets the version in src/gnoblin-version.json. The packages, the source bundle and gnoblin --version read it from there."

if [ "$make_tag" = true ]; then
    if [ "$sign" = true ]; then
        git tag -s "$tag" -m "Gnoblin $version"
    else
        git tag -a "$tag" -m "Gnoblin $version"
    fi
    echo "[release] tagged $tag"
fi

cat <<EOF
[release] Next steps. Nothing has been pushed.
  1. git push origin $branch
  2. Wait for the Release workflow dry run on that commit to pass.
  3. git push origin $tag
EOF
