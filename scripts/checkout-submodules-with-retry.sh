#!/usr/bin/env bash
# Fetch submodules, retrying GNOME GitLab failures. With no arguments, fetch every submodule. Arguments are submodule paths.
# CI runs this before CMake exists, so it stays a plain script. The pinned refs and the arguments are the same on every
# attempt, and a persistent failure still fails the caller.
set -euo pipefail

max_attempts=5
for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    if git submodule update --init --recursive "$@"; then
        exit 0
    fi
    if ((attempt < max_attempts)); then
        delay=$((attempt * 15))
        echo "Command failed; retrying in ${delay}s (attempt ${attempt}/${max_attempts})" >&2
        sleep "$delay"
    fi
done
echo "Command failed after ${max_attempts} attempts" >&2
exit 1
