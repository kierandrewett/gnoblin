#!/usr/bin/env bash
# Probe one target image for the native libraries the workspace needs, and store the report.
#
# usage: scripts/native-probe.sh ID [IMAGE]
#
#   ID     a target id from design/package-targets.json (`debian-12`), or `alpine-edge`
#   IMAGE  the registry name; by default the image the target names
#
# Run it as root on an Agent37 instance, from the repository root, with XTASK naming a built `xtask` binary (the
# script runs as root and must not run cargo, which would leave root-owned files in the target directory):
#
#   cargo build -p xtask && sudo XTASK=target/debug/xtask scripts/native-probe.sh debian-12
#
# What it does, each step recorded in the report's `[run]` table with its exit code:
#
#   1. unpacks the image with scripts/oci-rootfs.py into $NATIVE_PROBE_ROOTS/ID (no container runtime is needed);
#   2. installs, inside it with chroot, the packages `xtask native-probe packages FAMILY` lists for the image's family;
#   3. reads the root with `xtask native-probe check` and writes tests/native-probe/results/ID.toml.
#
# An image that cannot be unpacked gets a report that says so and why (`unprobed`), never a guess. A package the
# repositories do not have is not an error: the report lists it as absent and the library as missing, which is the
# finding. The root is deleted afterwards because the disk of an instance is small. NixOS has no packages to install
# into a root: the nixpkgs attributes are evaluated with the image's own `nix` instead.
set -uo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir" || exit 2
id="${1:?usage: scripts/native-probe.sh ID [IMAGE]}"
xtask="${XTASK:-target/debug/xtask}"
roots="${NATIVE_PROBE_ROOTS:-/home/node/probe-roots}"
out="${NATIVE_PROBE_OUT:-tests/native-probe/results}"
root="$roots/$id"
mkdir -p "$out" "$roots"

image="${2:-}"
if [ -z "$image" ]; then
    image="$("$xtask" native-probe image --image "$id")" || {
        echo "native-probe: $id is not a target" >&2
        exit 2
    }
fi

started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
host="$(uname -srm)"
box="${AGENT37_INSTANCE_ID:-unknown}"
common=(--meta "date=$started" --meta "host=$host" --meta "box=$box" --meta "xtask=$xtask native-probe check")

unprobed() {
    "$xtask" native-probe unprobed --image "$id" --image-name "$image" --reason "$1" "${common[@]}" "${@:2}" >"$out/$id.toml"
    echo "native-probe: $id not probed: $1"
    rm -rf "$root"
    exit 0
}

rm -rf "$root"

# ---- Nix: evaluate the attributes instead of installing ------------------------------------------------------------
case "$id" in
    nixos-*)
        channel="${id#nixos-}"
        [ "$channel" = unstable ] && ref="nixos-unstable" || ref="nixos-$channel"
        fetch_log="$(mktemp)"
        python3 scripts/oci-rootfs.py "$image" "$root" >"$fetch_log" 2>&1
        fetch_exit=$?
        if [ "$fetch_exit" -ne 0 ]; then
            unprobed "the image could not be unpacked (exit $fetch_exit): $(tail -n 1 "$fetch_log")" --meta "fetch-exit=$fetch_exit"
        fi
        nix_bin="$(chroot "$root" /bin/sh -c 'ls /nix/store/*-nix-*/bin/nix 2>/dev/null | head -n 1' 2>/dev/null)"
        cp /etc/resolv.conf "$root/etc/resolv.conf" 2>/dev/null || true
        versions="$(mktemp)"
        "$xtask" native-probe packages nix | while read -r attribute; do
            value="$(chroot "$root" "$nix_bin" --extra-experimental-features 'nix-command flakes' --option sandbox false \
                eval --raw "github:NixOS/nixpkgs/$ref#$attribute.version" 2>/dev/null)"
            [ -n "$value" ] && echo "$attribute $value"
        done >"$versions"
        if [ ! -s "$versions" ]; then
            unprobed "the image unpacked but its nix could not evaluate nixpkgs $ref here (no attribute answered); nix binary: ${nix_bin:-none}" \
                --meta "fetch-exit=$fetch_exit"
        fi
        "$xtask" native-probe check --versions "$versions" --image "$id" --image-name "$image" --os-id nixos \
            --os-version "$channel" "${common[@]}" --meta "fetch-exit=$fetch_exit" \
            --meta "install=nix eval --raw github:NixOS/nixpkgs/$ref#ATTRIBUTE.version (sandbox off)" >"$out/$id.toml"
        echo "native-probe: $id probed ($(wc -l <"$versions") attributes evaluated)"
        rm -rf "$root" "$versions"
        exit 0
        ;;
esac

# ---- Everything else: unpack, install, read -----------------------------------------------------------------------
fetch_log="$(mktemp)"
python3 scripts/oci-rootfs.py "$image" "$root" >"$fetch_log" 2>&1
fetch_exit=$?
if [ "$fetch_exit" -ne 0 ]; then
    unprobed "the image could not be unpacked (exit $fetch_exit): $(tail -n 1 "$fetch_log")" --meta "fetch-exit=$fetch_exit"
fi
family="$("$xtask" native-probe family --root "$root")" || unprobed "the unpacked image names no package family" --meta "fetch-exit=$fetch_exit"
mapfile -t packages < <("$xtask" native-probe packages "$family")
cp /etc/resolv.conf "$root/etc/resolv.conf" 2>/dev/null || true
. "$root/etc/os-release" 2>/dev/null || true
os_id="${ID:-}"
failed=()

# Runs one command inside the root; every step prints what it ran, so a log shows the sequence.
in_root() { chroot "$root" /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME=/root DEBIAN_FRONTEND=noninteractive "$@"; }

install_all() {
    local pm_exit=0
    case "$family" in
        apt)
            if ! in_root apt-get -o APT::Sandbox::User=root update; then
                if [ "$os_id" = debian ]; then
                    # A release past its end of life moves to the archive; the image still points at the live mirror.
                    in_root sed -i 's|deb.debian.org|archive.debian.org|g; s|security.debian.org[^ ]*|archive.debian.org/debian-security|g; /-updates/d' \
                        /etc/apt/sources.list /etc/apt/sources.list.d/debian.sources 2>/dev/null || true
                    in_root apt-get -o Acquire::Check-Valid-Until=false -o APT::Sandbox::User=root update || return 1
                else
                    return 1
                fi
            fi
            in_root apt-get -o APT::Sandbox::User=root install -y --no-install-recommends "${packages[@]}" && return 0
            for package in "${packages[@]}"; do
                in_root apt-get -o APT::Sandbox::User=root install -y --no-install-recommends "$package" || failed+=("$package")
            done
            ;;
        dnf)
            # Rocky: -devel packages live in CRB (9, 10) or PowerTools (8), and libseat in EPEL.
            case "$os_id" in
                rocky | rhel | centos)
                    in_root dnf -y --nogpgcheck --setopt=tsflags=nocaps install dnf-plugins-core epel-release || true
                    in_root dnf config-manager --set-enabled crb 2>/dev/null || in_root dnf config-manager --set-enabled powertools 2>/dev/null || true
                    ;;
            esac
            # No signature checks: gpg needs /proc, which a chroot on an Agent37 instance does not have ("GPGME: Invalid crypto
            # engine"), and the check is of the mirror's signature, not of what this probe reads.
            in_root dnf -y --nogpgcheck --setopt=repo_gpgcheck=0 --setopt=tsflags=nocaps --setopt=strict=0 --setopt=install_weak_deps=False install "${packages[@]}"
            pm_exit=$?
            for package in "${packages[@]}"; do
                in_root rpm -q "$package" >/dev/null 2>&1 || failed+=("$package")
            done
            return "$pm_exit"
            ;;
        zypper)
            in_root zypper --non-interactive --no-gpg-checks refresh || true
            in_root zypper --non-interactive --no-gpg-checks install --no-recommends "${packages[@]}"
            pm_exit=$?
            for package in "${packages[@]}"; do
                in_root rpm -q "$package" >/dev/null 2>&1 || failed+=("$package")
            done
            return "$pm_exit"
            ;;
        pacman)
            in_root pacman-key --init >/dev/null 2>&1 && in_root pacman-key --populate archlinux >/dev/null 2>&1 || true
            # A full upgrade (-Syu): the image is a snapshot, and installing a package from the current repository against an
            # older systemd-libs fails with "breaks dependency", which would read as a package that does not exist.
            in_root pacman -Syu --noconfirm --needed --disable-sandbox "${packages[@]}" && return 0
            for package in "${packages[@]}"; do
                in_root pacman -S --noconfirm --needed --disable-sandbox "$package" || failed+=("$package")
            done
            ;;
        apk)
            in_root apk add --no-cache "${packages[@]}" && return 0
            for package in "${packages[@]}"; do
                in_root apk add --no-cache "$package" || failed+=("$package")
            done
            ;;
    esac
    return 0
}

install_all >"$roots/$id.install.log" 2>&1
install_exit=$?
failed_list="${failed[*]:-none}"
"$xtask" native-probe check --root "$root" --image "$id" --image-name "$image" "${common[@]}" \
    --meta "fetch=python3 scripts/oci-rootfs.py $image" --meta "fetch-exit=$fetch_exit" \
    --meta "install=$family install of the packages in tests/native-probe/libraries.toml" \
    --meta "install-exit=$install_exit" --meta "packages-not-installed=$failed_list" >"$out/$id.toml"
check_exit=$?
echo "native-probe: $id probed: fetch $fetch_exit, install $install_exit, check $check_exit, not installed: $failed_list"
rm -rf "$root"
exit "$check_exit"
