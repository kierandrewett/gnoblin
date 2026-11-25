#!/usr/bin/env bash
# Make room on a GitHub-hosted Ubuntu runner before a full workspace build.
#
# A hosted runner starts with about 14 GB free, and a debug build of the workspace with its test targets needs more:
# the workspace job died with "No space left on device" before its tests ran. The runner image carries toolchains no
# job here uses (.NET, Android, GHC, Swift, CodeQL, the hosted tool cache) and pre-pulled container images; removing
# them gives back about 30 GB.
#
# It refuses to run anywhere but a hosted runner (GITHUB_ACTIONS=true and RUNNER_ENVIRONMENT=github-hosted), so a
# developer or a self-hosted runner that runs it by mistake loses nothing.
set -euo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

if [ "${GITHUB_ACTIONS:-}" != true ] || [ "${RUNNER_ENVIRONMENT:-}" != github-hosted ]; then
    echo "[free-disk] not a GitHub-hosted runner; nothing removed" >&2
    exit 0
fi

echo "[free-disk] before: $(df -h --output=avail / | tail -1 | tr -d ' ') free"
sudo rm -rf \
    /usr/share/dotnet \
    /usr/local/lib/android \
    /opt/ghc \
    /usr/local/.ghcup \
    /usr/share/swift \
    /opt/hostedtoolcache/CodeQL \
    /usr/local/share/powershell \
    /usr/local/share/chromium \
    /usr/local/lib/node_modules
sudo docker image prune --all --force >/dev/null 2>&1 || true
echo "[free-disk] after: $(df -h --output=avail / | tail -1 | tr -d ' ') free"
