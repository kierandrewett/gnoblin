#!/usr/bin/env bash
# Type-checks compat.ts against the old declarations (crates/gnoblin-std-impl/tests/ORACLE.md, section 5).
set -euo pipefail
cd "$(dirname -- "${BASH_SOURCE[0]}")"
npx --yes --package=typescript@7.0.2 tsc --project tsconfig.json --pretty false
