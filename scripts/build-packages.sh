#!/usr/bin/env bash
# Write the Gnoblin packages from a prefix with pkgsmith.
#
# usage: scripts/build-packages.sh [--prefix DIR] [--output DIR] [--version SEMVER] [--formats LIST] [--release N] [--fedora N] [--list]
#
# DIR is the prefix that scripts/build-prefix.sh made. The script runs pkgsmith once for each script in pkgsmith/ and
# writes every package into the output folder (default: dist):
#
#   pkgsmith/runtime.star       gnoblin and xdg-desktop-portal-gnoblin, which hold compiled code
#   pkgsmith/integrations.star  gnoblin-geoclue-integration and gnoblin-ananicy-rules, which hold configuration only
#
# --version is a SemVer number. The default is the version of the tree (src/gnoblin-version.json), which is also the
# version of a nightly build once scripts/apply-nightly-version.py has written it. --formats limits the formats, for
# example rpm,deb. The architecture and the C library are read from the compositor in the prefix, so a prefix built
# on arm64 gets arm64 packages on any machine that runs this script, and a prefix built on Alpine (musl) gets apk packages.
#
# --list writes no package and prints the files that each package would hold, for a review of what the prefix gives
# each package.
#
# pkgsmith must be on PATH. scripts/install-pkgsmith.sh builds the pinned one.
set -euo pipefail

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

prefix="$root/prefix"
output="$root/dist"
version=""
formats=""
release=1
fedora=45
list=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            prefix="${2:?--prefix needs a directory}"
            shift 2
            ;;
        --output)
            output="${2:?--output needs a directory}"
            shift 2
            ;;
        --version)
            version="${2:?--version needs a version}"
            shift 2
            ;;
        --formats)
            formats="${2:?--formats needs a comma separated list}"
            shift 2
            ;;
        --release)
            release="${2:?--release needs a number}"
            shift 2
            ;;
        --fedora)
            fedora="${2:?--fedora needs a release number}"
            shift 2
            ;;
        --list)
            list=true
            shift
            ;;
        *)
            echo "usage: $0 [--prefix DIR] [--output DIR] [--version SEMVER] [--formats LIST] [--release N] [--fedora N] [--list]" >&2
            exit 2
            ;;
    esac
done
prefix="$(realpath -- "$prefix")"
output="$(realpath -m -- "$output")"
version="${version:-$(python3 scripts/gnoblin-version.py get version)}"
command -v pkgsmith >/dev/null || {
    echo "[packages] pkgsmith is not on PATH. Run: PATH=\"\$(scripts/install-pkgsmith.sh):\$PATH\"" >&2
    exit 1
}

# The prefix says what it is for: the machine type and the C library of the compositor decide the package architecture
# and which formats can carry it. pkgsmith does not look inside the files.
compositor="$prefix/usr/lib/gnoblin/bin/gnoblin"
[ -x "$compositor" ] || {
    echo "[packages] $compositor is missing. Build the prefix first: scripts/build-prefix.sh --prefix $prefix" >&2
    exit 1
}
machine="$(od -An -tx2 -j18 -N2 -- "$compositor" | tr -d ' ')"
case "$machine" in
    003e) arch=amd64 ;;
    00b7) arch=arm64 ;;
    *)
        echo "[packages] the compositor has an unknown machine type 0x$machine" >&2
        exit 1
        ;;
esac
if grep -aq 'ld-musl' -- "$compositor"; then
    libc=musl
else
    libc=glibc
fi
echo "[packages] version $version, $arch, $libc"

# pkgsmith does not look inside the programs, so the libraries they need go to the scripts as arguments. The portal
# backend has its own list. A library that the prefix carries itself, such as Mutter's, needs no package.
is_elf() {
    [ "$(head -c 4 -- "$1" | od -An -tx1 | tr -d ' ')" = 7f454c46 ]
}
mapfile -t provided < <(find "$prefix/usr/lib/gnoblin/lib" -name '*.so*' -printf '%f\n' | sort -u)
core_programs=()
portal_programs=()
while IFS= read -r -d '' file; do
    is_elf "$file" || continue
    case "$file" in
        */libexec/xdg-desktop-portal-gnoblin) portal_programs+=("$file") ;;
        *) core_programs+=("$file") ;;
    esac
done < <(find "$prefix/usr" -type f -perm /111 -print0)
[ "${#core_programs[@]}" -gt 0 ] && [ "${#portal_programs[@]}" -gt 0 ] || {
    echo "[packages] found no programs in $prefix/usr" >&2
    exit 1
}
# On arm64 the dynamic loader (ld-linux-aarch64.so.1) is a NEEDED entry too. It comes with glibc, like libc.so.6, which
# already stands for the C library, so it is not a library of its own.
needed() {
    readelf -d -- "$@" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sed '/^ld-linux/d' | sort -u |
        grep -v -x -F -f <(printf '%s\n' "${provided[@]}") | paste -sd, -
}
needed_gnoblin="$(needed "${core_programs[@]}")"
needed_portal="$(needed "${portal_programs[@]}")"
# The newest glibc symbol version that any program uses is the oldest glibc that runs them.
glibc="$(objdump -T -- "${core_programs[@]}" "${portal_programs[@]}" 2>/dev/null | grep -o '(GLIBC_[0-9][0-9.]*)' |
    tr -d '()' | sed 's/^GLIBC_//' | sort -V | tail -n 1 || true)"
# The portal backend takes the version of its GNOME source, which pkgsmith needs as three numbers.
portal_version="$(python3 scripts/gnome-versions.py get xdg-desktop-portal-gnome version)"
while [ "$(tr -cd . <<<"$portal_version" | wc -c)" -lt 2 ]; do
    portal_version="$portal_version.0"
done
echo "[packages] glibc ${glibc:-none}, portal $portal_version"
echo "[packages] gnoblin needs: $needed_gnoblin"
echo "[packages] portal needs:  $needed_portal"

: "${SOURCE_DATE_EPOCH:=$(git log -1 --format=%ct 2>/dev/null || stat -c %Y build.sh)}"
export SOURCE_DATE_EPOCH

format_args=()
if [ -n "$formats" ]; then
    format_args=(-F "$formats")
fi
for script in runtime integrations; do
    script_args=(
        -f "pkgsmith/$script.star" --prefix "$prefix"
        --build-arg "VERSION=$version" --build-arg "ARCH=$arch" --build-arg "LIBC=$libc"
        --build-arg "RELEASE=$release" --build-arg "FEDORA=$fedora" --build-arg "GLIBC=$glibc"
        --build-arg "PORTAL_VERSION=$portal_version" --build-arg "NEEDED_GNOBLIN=$needed_gnoblin"
        --build-arg "NEEDED_PORTAL=$needed_portal"
    )
    if "$list"; then
        pkgsmith files . "${script_args[@]}"
    else
        mkdir -p "$output"
        pkgsmith build . "${script_args[@]}" -o "$output" --progress plain "${format_args[@]}"
    fi
done
if ! "$list"; then
    ls -l "$output"
fi
