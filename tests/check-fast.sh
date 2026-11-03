#!/usr/bin/env bash
# The fast checks: manifests, patch metadata, script syntax, and the configuration and package tests.
# They need no build and no session. Run them with "make check".
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

run() {
    printf '+ %s\n' "$*"
    "$@"
}

run ./scripts/gnome-versions.py check
run ./scripts/manage-patches.py check

printf '+ bash -n scripts/*.sh tests/*.sh tests/packages/*.sh src/tools/*.sh\n'
for file in scripts/*.sh tests/*.sh tests/packages/*.sh src/tools/*.sh; do
    bash -n "$file"
done

printf '+ python3 -m py_compile scripts/*.py tests/*.py tests/packages/*.py\n'
cache="$(mktemp -d)"
trap 'rm -rf -- "$cache"' EXIT
PYTHONPYCACHEPREFIX="$cache" python3 -m py_compile scripts/*.py tests/*.py tests/packages/*.py

run ./tests/test-log-diagnostics.sh
run ./tests/test-secure-state.sh
run ./tests/config-seed.test.sh
run python3 tests/frame-renderer-policy.test.py
run python3 tests/session-environment.test.py
run python3 tests/package-isolation.test.py
run python3 tests/pkgsmith-packages.test.py
run python3 tests/pkgrepo.test.py
run python3 tests/supported-targets.test.py
run python3 tests/packages/selftest/selftest.py
run python3 tests/pipeline/stages.test.py
run python3 tests/pipeline/issues.test.py
run python3 tests/graphical/harness.test.py
run python3 tests/graphical/arch-arm-image.test.py
run python3 tests/check-build-deps.test.py
run python3 tests/test_generate_mutter_keybinding_catalog.py
run ./tests/test-config.sh
