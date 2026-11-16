#!/usr/bin/env bash
# Proves the editor files `gnoblinctl types init` writes: that the real library is the types.
#
# It writes a configuration directory with the built `gnoblinctl`, puts the starter `shell.ts` and the programs of
# tests/js-types in it (the ones that must compile, and the wrong ones that must fail with a named error), runs the
# native compiler over the directory's own tsconfig.json, and then asks the compiler API what an editor asks:
# where `gnoblin-std/scene` resolves, what go-to-definition opens, which class a class extends, what hover shows
# (scripts/js-types-check.mjs).
#
# GNOBLINCTL names a built binary; without it the script runs `cargo run -p gnoblinctl`. The compiler is the one
# scripts/js-typecheck.sh pins. The API comes from the TypeScript 6.0.3 package, the one scripts/js-lint.sh pins,
# because the 7.0.2 package exposes no API.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/gnoblin-js-types.XXXXXX")"
trap 'rm -rf "$work"' EXIT
config="$work/config"

cd "$repo_root"
if [ -n "${GNOBLINCTL:-}" ]; then
    "$GNOBLINCTL" types init "$config"
else
    cargo run --quiet -p gnoblinctl -- types init "$config"
fi

# A user's directory: shell.ts and a few files beside it. The wrong programs sit in a directory of their own so that
# the checker can tell which file a line belongs to.
cp data/js/starter/shell.ts "$config/shell.ts"
cp tests/js-types/ok/*.ts "$config/"
mkdir "$config/bad"
cp tests/js-types/bad/*.ts "$config/bad/"

# `tsc` exits 2 when it reports errors, and the wrong programs make it report some; the checker reads the report.
npx --yes --package=typescript@7.0.2 tsc --project "$config/tsconfig.json" --pretty false >"$work/tsc.out" || true

npm install --prefix "$work/api" --no-audit --no-fund --silent typescript@6.0.3
node scripts/js-types-check.mjs "$config" "$work/tsc.out" "$work/api/node_modules/typescript/lib/typescript.js"
