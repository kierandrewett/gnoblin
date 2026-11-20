#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
temporary_config="$(mktemp "${TMPDIR:-/tmp}/gnoblin-js-tsconfig.XXXXXX.json")"
generated="$(mktemp -d "${TMPDIR:-/tmp}/gnoblin-js-generated.XXXXXX")"
trap 'rm -rf "$temporary_config" "$generated"' EXIT

# The generated modules of both packages (`gnoblin-core/generated`, `gnoblin-std/generated`) are committed nowhere: the generator renders
# them, as a build does, and the paths below find them beside the hand-written library.
(cd "$repo_root" && cargo xtask gen-sdks --out "$generated" --target typescript >/dev/null &&
    cargo xtask gen-sdks --out "$generated" --target object-types >/dev/null)

cat >"$temporary_config" <<EOF
{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2023",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "lib": ["ES2023"],
    "paths": {
      "gnoblin-core": ["$repo_root/data/js/gnoblin-core/index.ts"],
      "gnoblin-core/*": ["$repo_root/data/js/gnoblin-core/*", "$generated/js/gnoblin-core/*"],
      "gnoblin-std": ["$repo_root/data/js/gnoblin-std/index.ts"],
      "gnoblin-std/*": ["$repo_root/data/js/gnoblin-std/*", "$generated/js/gnoblin-std/*"]
    }
  },
  "include": [
    "$repo_root/data/js/**/*.ts",
    "$generated/js/**/*.ts"
  ]
}
EOF

npx --yes --package=typescript@7.0.2 tsc --project "$temporary_config"

# What a user sees is checked on the files `gnoblinctl types init` writes, with the starter as the user's `shell.ts`.
"$repo_root/scripts/js-types-check.sh"
