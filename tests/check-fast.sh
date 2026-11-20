#!/usr/bin/env bash
# The fast checks of the scripts: shell and Python syntax, and the tests of the package repository tool, the RPM target
# probe, the reference session matrix, the incident resource reporter and the contract recording.
# They need no build and no session. Run them with `just script-tests`.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

run() {
    printf '+ %s\n' "$*"
    "$@"
}

printf '+ bash -n scripts/*.sh tests/*.sh\n'
for file in scripts/*.sh tests/*.sh; do
    bash -n "$file"
done

printf '+ python3 -m py_compile scripts/*.py tests/*.py\n'
cache="$(mktemp -d)"
trap 'rm -rf -- "$cache"' EXIT
PYTHONPYCACHEPREFIX="$cache" python3 -m py_compile scripts/*.py tests/*.py

run python3 tests/pkgrepo.test.py
run python3 tests/rpm-target-probe.test.py
run python3 tests/reference-session-matrix.test.py
run python3 tests/incident-resources.test.py
run python3 tests/contract-record.test.py
