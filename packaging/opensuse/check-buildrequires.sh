#!/usr/bin/env bash
# Resolve the repository-owned prerequisites for the openSUSE adapter.
# The private Gnoblin packages are intentionally excluded: they are built in
# dependency order and supplied by the eventual OBS project, not Tumbleweed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPECS=(
    "$ROOT/packaging/opensuse/gsettings-desktop-schemas.spec"
    "$ROOT/packaging/opensuse/mutter.spec"
    "$ROOT/packaging/opensuse/gnome-shell.spec"
    "$ROOT/packaging/opensuse/gnoblin.spec"
)

command -v rpmspec >/dev/null
command -v zypper >/dev/null

requirements="$(mktemp)"
cleanup() {
    rm -f -- "$requirements"
}
trap cleanup EXIT

for spec in "${SPECS[@]}"; do
    rpmspec -P --without gnoblin_stack "$spec" >/dev/null
    rpmspec -q --buildrequires --without gnoblin_stack "$spec"
done | LC_ALL=C sort -u >"$requirements"

if [[ -s "$requirements" ]]; then
    # zypper understands RPM capabilities such as pkgconfig(gtk4), including
    # their version constraints.  Do not turn this into a package-name list:
    # those names vary across Tumbleweed snapshots.
    xargs -r -d '\n' zypper --non-interactive install --dry-run --no-recommends <"$requirements"
fi

printf 'PASS: openSUSE Tumbleweed repository BuildRequires resolve\n'
