#!/usr/bin/env bash
set -euo pipefail

max_attempts=5
git config --global --add safe.directory "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"

for attempt in $(seq 1 "$max_attempts"); do
    if git submodule update --init --force --depth=1 --recursive; then
        exit 0
    fi

    if ((attempt < max_attempts)); then
        delay=$((attempt * 15))
        echo "Submodule fetch failed; retrying in ${delay}s (attempt ${attempt}/${max_attempts})" >&2
        sleep "$delay"
    fi
done

echo "Failed to fetch submodules after ${max_attempts} attempts" >&2
exit 1
