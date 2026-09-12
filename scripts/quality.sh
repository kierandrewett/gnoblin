#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:-lint}"
if (($#)); then shift; fi
case "$mode" in
    lint) stage=pre-commit ;;
    format) stage=manual ;;
    *)
        echo "Usage: $0 {lint|format} [pre-commit run options, e.g. --files path]" >&2
        exit 2
        ;;
esac
if ! command -v uv >/dev/null; then
    echo "Install uv (https://docs.astral.sh/uv/getting-started/installation/), then retry." >&2
    exit 2
fi
if (($# == 0)); then set -- --all-files; fi
exec uv tool run --from pre-commit==4.6.2 pre-commit run --hook-stage "$stage" "$@"
