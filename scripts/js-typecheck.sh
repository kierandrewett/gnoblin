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
      "gnoblin": ["$repo_root/data/js/gnoblin/index.ts"],
      "gnoblin/*": ["$repo_root/data/js/gnoblin/*"],
      "gnoblin/host*": ["$repo_root/crates/gnoblin-js/js/gnoblin.ts"]
    }
  },
  "include": [
    "$repo_root/data/js/**/*.ts",
    "$repo_root/data/js/starter/**/*.ts",
    "$repo_root/crates/gnoblin-js/js/*.ts"
  ]
}
EOF

npx --yes --package=typescript@7.0.2 tsc --project "$temporary_config"
