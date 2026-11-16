#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
temporary_config="$(mktemp "${TMPDIR:-/tmp}/gnoblin-js-tsconfig.XXXXXX.json")"
trap 'rm -f "$temporary_config"' EXIT

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
      "gnoblin-core/*": ["$repo_root/data/js/gnoblin-core/*"],
      "gnoblin-std": ["$repo_root/data/js/gnoblin-std/index.ts"],
      "gnoblin-std/*": ["$repo_root/data/js/gnoblin-std/*"]
    }
  },
  "include": [
    "$repo_root/data/js/**/*.ts"
  ],
  "exclude": ["$repo_root/data/js/starter"]
}
EOF

npx --yes --package=typescript@7.0.2 tsc --project "$temporary_config"

# What a user sees is checked on the files `gnoblinctl types init` writes, with the starter as the user's `shell.ts`.
"$repo_root/scripts/js-types-check.sh"
