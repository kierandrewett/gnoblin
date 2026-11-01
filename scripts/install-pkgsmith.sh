#!/usr/bin/env bash
# Build pkgsmith and its format plugins at the commit pinned in scripts/pkgsmith.ref, and print the folder that holds
# the binary.
#
# usage: scripts/install-pkgsmith.sh [--dir DIR]
#
# pkgsmith is the tool that writes the rpm, deb, Arch, Alpine and tar packages from the prefix that
# scripts/build-prefix.sh makes. It has no release with the script-based interface yet, so CI builds the pinned commit.
# The plugins are shared libraries beside the binary, where pkgsmith finds them. DIR defaults to
# ~/.cache/gnoblin/pkgsmith. Run the result with PATH="$(scripts/install-pkgsmith.sh):$PATH" pkgsmith plan.
#
# Needs git and a Rust toolchain (cargo). To move to a newer pkgsmith, write its commit into scripts/pkgsmith.ref.
set -euo pipefail

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
ref="$(tr -d '[:space:]' <"$root/scripts/pkgsmith.ref")"
[[ "$ref" =~ ^[0-9a-f]{40}$ ]] || {
    echo "[pkgsmith] scripts/pkgsmith.ref must hold a 40-digit commit, not: $ref" >&2
    exit 1
}
dir="${XDG_CACHE_HOME:-$HOME/.cache}/gnoblin/pkgsmith"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dir)
            dir="${2:?--dir needs a directory}"
            shift 2
            ;;
        *)
            echo "usage: $0 [--dir DIR]" >&2
            exit 2
            ;;
    esac
done
dir="$(realpath -m -- "$dir")"
bin="$dir/$ref/bin"
if [ -x "$bin/pkgsmith" ]; then
    echo "$bin"
    exit 0
fi

for tool in git cargo; do
    command -v "$tool" >/dev/null || {
        echo "[pkgsmith] missing $tool" >&2
        exit 1
    }
done
source_dir="$dir/$ref/source"
rm -rf -- "${dir:?}/$ref"
mkdir -p "$source_dir" "$bin"
git -C "$source_dir" init --quiet
git -C "$source_dir" fetch --quiet --depth 1 https://github.com/kierandrewett/pkgsmith.git "$ref"
git -C "$source_dir" checkout --quiet FETCH_HEAD
(cd "$source_dir" && cargo build --release --workspace --locked 1>&2)
install -m 0755 "$source_dir/target/release/pkgsmith" "$bin/pkgsmith"
for plugin in "$source_dir"/target/release/libpkgsmith_plugin_*.so; do
    name="${plugin##*/libpkgsmith_plugin_}"
    install -m 0755 "$plugin" "$bin/pkgsmith-plugin-$name"
done
rm -rf -- "$source_dir"
echo "$bin"
