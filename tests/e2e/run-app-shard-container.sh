#!/usr/bin/env bash
set -euo pipefail

: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE must be set}"
: "${SHARD_INDEX:?SHARD_INDEX must be set}"
: "${ARTIFACT_DIR:?ARTIFACT_DIR must be set}"

mkdir -p "$ARTIFACT_DIR"

# Flatpak needs nested user/mount/network namespaces. Configure the disposable
# GitHub runner before entering Fedora, then keep the test environment in the
# same privileged Fedora container that receives the built Gnoblin prefix.
docker run --rm --privileged \
    --mount "type=bind,src=$GITHUB_WORKSPACE,dst=$GITHUB_WORKSPACE" \
    --mount "type=bind,src=$ARTIFACT_DIR,dst=$ARTIFACT_DIR" \
    --workdir "$GITHUB_WORKSPACE" \
    --env GITHUB_WORKSPACE \
    --env SHARD_INDEX \
    --env ARTIFACT_DIR \
    fedora:44 \
    bash "$GITHUB_WORKSPACE/tests/e2e/run-app-shard-in-fedora.sh"
