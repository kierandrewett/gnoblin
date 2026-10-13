#!/usr/bin/env bash
# Build source RPMs from previously prepared Gnoblin release sources.
set -euo pipefail
if [[ $# != 3 || "$1" != xdg-desktop-portal-gnoblin && "$1" != gnoblin ]]; then
    echo "Usage: $0 <xdg-desktop-portal-gnoblin|gnoblin> <prepared-source-directory> <output-directory>" >&2
    exit 2
fi
project="$1"
source_dir="$(realpath "$2")"
output_dir="$(realpath -m "$3")"
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
command -v rpmbuild >/dev/null
command -v rpmspec >/dev/null
mkdir -p "$output_dir"
spec="$repo_dir/packaging/rpm/$project.spec"
# Do not use spectool to download Source0: upstream archives lack our patches.
expanded_spec="$(rpmspec -P "$spec")"
if [[ "$project" == gnoblin ]]; then
    version="$("$repo_dir/scripts/gnoblin-version.py" get version)"
    source_archive="$source_dir/gnoblin-$version-source.tar.xz"
    if [[ ! -f "$source_archive" ]]; then
        echo "Missing complete Gnoblin source bundle: $source_archive" >&2
        exit 1
    fi
fi
while IFS= read -r source; do
    name="${source##*/}"
    if [[ ! -f "$source_dir/$name" ]]; then
        echo "Missing prepared source: $source_dir/$name" >&2
        echo "Prepare sources with scripts/make-tarball.sh in a clean release checkout." >&2
        exit 1
    fi
done < <(printf '%s\n' "$expanded_spec" | sed -n 's/^Source[0-9]*:[[:space:]]*//p')
rpmbuild -bs --define "_sourcedir $source_dir" --define "_srcrpmdir $output_dir" "$spec"
