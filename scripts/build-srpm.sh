#!/usr/bin/env bash
# Build source RPMs from previously prepared Gnoblin release sources.
set -euo pipefail
if [[ $# != 3 || "$1" != mutter && "$1" != gnome-shell ]]; then
    echo "Usage: $0 <mutter|gnome-shell> <prepared-source-directory> <output-directory>" >&2
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
while IFS= read -r source; do
    name="${source##*/}"
    if [[ ! -f "$source_dir/$name" ]]; then
        echo "Missing prepared source: $source_dir/$name" >&2
        echo "Prepare sources with scripts/make-tarball.sh in a clean release checkout." >&2
        exit 1
    fi
done < <(printf '%s\n' "$expanded_spec" | sed -n 's/^Source[0-9]*:[[:space:]]*//p')
rpmbuild -bs --define "_sourcedir $source_dir" --define "_srcrpmdir $output_dir" "$spec"
