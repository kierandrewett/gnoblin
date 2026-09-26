#!/usr/bin/env bash
# Resolve the repository-owned prerequisites for the openSUSE adapter.
# The private Gnoblin packages are intentionally excluded: they are built in
# dependency order and supplied by the eventual OBS project, not Tumbleweed.
set -euo pipefail

install=0
compatibility_runtime=0
while (($#)); do
    case "$1" in
        --install) install=1 ;;
        --compat-runtime) compatibility_runtime=1 ;;
        *)
            echo "Usage: $0 [--install] [--compat-runtime]" >&2
            exit 2
            ;;
    esac
    shift
done

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

rpmspec_args=()
if ((compatibility_runtime)); then
    # Leap 15.6's RPM 4.14 lacks `rpmspec --with`. Defining the bcond macro
    # makes this probe use the installed local compatibility RPM instead of
    # trying to resolve GNOME 51 from the host repository.
    rpmspec_args=(--define "_with_gnoblin_compat_runtime 1")
fi

for spec in "${SPECS[@]}"; do
    # `%bcond_with gnoblin_stack` is disabled by default.  Do not pass
    # rpmspec's `--without` convenience option here: RPM 4.14 in Leap 15.6
    # predates that option even though it understands `%bcond_with`.
    rpmspec -P "${rpmspec_args[@]}" "$spec" >/dev/null
    rpmspec -q --buildrequires "${rpmspec_args[@]}" "$spec"
done | LC_ALL=C sort -u >"$requirements"

if [[ -s "$requirements" ]]; then
    # zypper understands RPM capabilities such as pkgconfig(gtk4), including
    # their version constraints.  Do not turn this into a package-name list:
    # those names vary across Tumbleweed snapshots.
    if ((install)); then
        installed=0
        for attempt in 1 2 3; do
            if xargs -r -d '\n' zypper --non-interactive install --no-recommends <"$requirements"; then
                installed=1
                break
            fi

            if ((attempt < 3)); then
                printf 'Tumbleweed install failed; refreshing repositories before retry %s/3\n' "$((attempt + 1))" >&2
                zypper --non-interactive refresh --force
                sleep "$((attempt * 10))"
            fi
        done
        ((installed))
    else
        xargs -r -d '\n' zypper --non-interactive install --dry-run --no-recommends <"$requirements"
    fi
fi

printf 'PASS: openSUSE repository BuildRequires %s%s\n' \
    "$([[ $install == 1 ]] && echo installed || echo resolve)" \
    "$( ((compatibility_runtime)) && echo ' with private runtime' || true)"
