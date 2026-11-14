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

if diagnostics="$(npx --yes --package=typescript@7.0.2 tsc --project "$temporary_config" 2>&1)"; then
    tsc_status=0
else
    tsc_status=$?
fi
if [[ -n "$diagnostics" ]]; then
    printf '%s\n' "$diagnostics"
fi

if [[ ! -f "$repo_root/scripts/js-typecheck.baseline" ]]; then
    exit "$tsc_status"
fi

declare -A actual_counts=()
while IFS= read -r line; do
    if [[ "$line" =~ ^([^\(]+)\([0-9]+,[0-9]+\):\ error\ TS[0-9]+: ]]; then
        file="${BASH_REMATCH[1]#"$repo_root"/}"
        actual_counts["$file"]=$((${actual_counts["$file"]:-0} + 1))
    fi
done <<<"$diagnostics"

baseline_failed=false
while IFS=' ' read -r file expected; do
    [[ -z "$file" || "$file" == \#* ]] && continue
    actual="${actual_counts["$file"]:-0}"
    if ((actual > expected)); then
        printf 'TypeScript diagnostics increased for %s: %s > baseline %s\n' "$file" "$actual" "$expected" >&2
        baseline_failed=true
    elif ((actual < expected)); then
        printf 'TypeScript diagnostics decreased for %s: %s < baseline %s\n' "$file" "$actual" "$expected"
    fi
    unset 'actual_counts[$file]'
done <"$repo_root/scripts/js-typecheck.baseline"

for file in "${!actual_counts[@]}"; do
    printf 'TypeScript diagnostics have no baseline for %s: %s\n' "$file" "${actual_counts[$file]}" >&2
    baseline_failed=true
done

if [[ "$baseline_failed" == true ]]; then
    exit 1
fi
exit 0
