#!/usr/bin/env bash
set -euo pipefail

: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE must be set}"
: "${SHARD_INDEX:?SHARD_INDEX must be set}"
: "${ARTIFACT_DIR:?ARTIFACT_DIR must be set}"

mkdir -p "$ARTIFACT_DIR"

# build-and-catalog runs in a GitHub Actions job container. Actions mounts the
# checkout at /__w/<repository>/<repository> there, and GNOME Shell embeds that
# absolute prefix in Config.PKGDATADIR. Reuse the same path in the Fedora app
# container so the relocated build remains runnable.
repo_name="${GITHUB_REPOSITORY##*/}"
: "${repo_name:?GITHUB_REPOSITORY must be set}"
container_workspace="/__w/$repo_name/$repo_name"

# Flatpak needs nested user/mount/network namespaces. Configure the disposable
# GitHub runner before entering Fedora. Disable the container seccomp and
# AppArmor profiles for bubblewrap, without granting privileged access or
# passing host GPU/audio devices into the Fedora test container.
docker run --rm \
    --security-opt seccomp=unconfined \
    --security-opt apparmor=unconfined \
    --mount "type=bind,src=$GITHUB_WORKSPACE,dst=$container_workspace" \
    --mount "type=bind,src=$ARTIFACT_DIR,dst=$ARTIFACT_DIR" \
    --workdir "$container_workspace" \
    --env GITHUB_WORKSPACE="$container_workspace" \
    --env SHARD_INDEX \
    --env ARTIFACT_DIR \
    --env TRACE_CRASH \
    --env APP_IDS \
    --env GNOBLIN_E2E_EXTRA_MONITOR_OVERRIDE \
    --env "GNOBLIN_E2E_FAILURE_POLICY=${GNOBLIN_E2E_FAILURE_POLICY:-strict}" \
    --env "GNOBLIN_E2E_REQUIRED_APP_IDS=${GNOBLIN_E2E_REQUIRED_APP_IDS:-}" \
    fedora:44 \
    bash "$container_workspace/tests/e2e/run-app-shard-in-fedora.sh"
