#!/usr/bin/env bash
# Resolve the repository-owned prerequisites for the openSUSE adapter.
# The private Gnoblin packages are intentionally excluded: they are built in
# dependency order and supplied by the eventual OBS project, not Tumbleweed.
set -euo pipefail

install=0
package=""
for arg in "$@"; do
    case "$arg" in
        --install) install=1 ;;
        mutter | gnoblin-portal | gnoblin)
            if [[ -n "$package" ]]; then
                echo "Pass exactly one package: mutter, gnoblin-portal, or gnoblin" >&2
                exit 2
            fi
            package="$arg"
            ;;
        *)
            echo "Usage: $0 <mutter|gnoblin-portal|gnoblin> [--install]" >&2
            exit 2
            ;;
    esac
done

if [[ -z "$package" ]]; then
    echo "Usage: $0 <mutter|gnoblin-portal|gnoblin> [--install]" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$package" in
    mutter) spec="$ROOT/packaging/opensuse/mutter.spec" ;;
    gnoblin-portal) spec="$ROOT/packaging/opensuse/gnoblin-portal.spec" ;;
    gnoblin) spec="$ROOT/packaging/opensuse/gnoblin.spec" ;;
esac

command -v rpmspec >/dev/null
command -v zypper >/dev/null

requirements="$(mktemp)"
cleanup() {
    rm -f -- "$requirements"
}
trap cleanup EXIT

rpmspec -P "$spec" >/dev/null
rpmspec -q --buildrequires "$spec" | LC_ALL=C sort -u >"$requirements"

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

printf 'PASS: openSUSE Tumbleweed %s BuildRequires %s\n' \
    "$package" "$([[ $install == 1 ]] && echo installed || echo resolve)"
