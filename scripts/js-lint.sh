#!/usr/bin/env bash
# Lints every tracked JS/TS file with the repo's eslint config.
#
# The pins below are the one place the pair is decided, and .pre-commit-config.yaml must carry the same versions
# (this script refuses to run when it does not). typescript-eslint 8.72.2 declares the peer range
# `typescript >=4.8.4 <6.1.0`, so eslint runs on TypeScript 6.0.3, the newest release inside that range, even
# though scripts/js-typecheck.sh checks types with the native 7.0.2 compiler: typescript-eslint loads the
# TypeScript *library* for its parser, and the 7.0.2 package's entry point (`exports["."]`) exposes only the version,
# not the compiler API.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

pins=(
    eslint@10.10.0
    typescript-eslint@8.72.2
    typescript@6.0.3
    @eslint/js@10.0.1
    eslint-plugin-svelte@3.23.1
    globals@17.13.0
    svelte@5.57.3
)

# The hook installs its own copy of the same packages, so a drifted pin would lint differently there.
for pin in "${pins[@]}"; do
    grep -qF "\"$pin\"" .pre-commit-config.yaml || {
        echo "js-lint: .pre-commit-config.yaml does not pin $pin" >&2
        exit 1
    }
done

package_args=()
for pin in "${pins[@]}"; do
    package_args+=("--package=$pin")
done

if [ "$#" -gt 0 ]; then
    files=("$@")
    npx --yes "${package_args[@]}" eslint --no-warn-ignored "${files[@]}"
else
    git ls-files -z '*.js' '*.mjs' '*.cjs' '*.ts' |
        xargs -0 npx --yes "${package_args[@]}" eslint --no-warn-ignored
fi
