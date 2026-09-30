#!/usr/bin/env bash
# Publish prepared source RPMs after each required private dependency is built.
set -euo pipefail
if [[ $# != 4 || "$1" != */* ]]; then
    echo "Usage: $0 <owner/project> <mutter.src.rpm> <portal.src.rpm> <gnoblin.src.rpm>" >&2
    exit 2
fi
project="$1"
mutter_srpm="$(realpath "$2")"
portal_srpm="$(realpath "$3")"
meta_srpm="$(realpath "$4")"
command -v copr-cli >/dev/null
chroots=(fedora-45-x86_64)

build_in_supported_fedora_chroots() {
    local package="$1"
    local chroot
    local arguments=()
    for chroot in "${chroots[@]}"; do
        arguments+=(--chroot "$chroot")
    done
    copr-cli build "${arguments[@]}" "$project" "$package"
}

for package in "$mutter_srpm" "$portal_srpm" "$meta_srpm"; do
    [[ -f "$package" ]] || {
        echo "Missing source RPM: $package" >&2
        exit 1
    }
    [[ "$(rpm -qp --qf '%{SOURCEPACKAGE}' "$package")" == 1 ]] || {
        echo "Not a source RPM: $package" >&2
        exit 1
    }
done
[[ "$(rpm -qp --qf '%{NAME}' "$mutter_srpm")" == gnoblin-mutter ]]
[[ "$(rpm -qp --qf '%{NAME}' "$portal_srpm")" == gnoblin-portal ]]
[[ "$(rpm -qp --qf '%{NAME}' "$meta_srpm")" == gnoblin ]]
# copr-cli waits by default. Build each dependency in all supported Fedora
# chroots before starting its dependent package; any failed chroot stops here.
build_in_supported_fedora_chroots "$mutter_srpm"
build_in_supported_fedora_chroots "$portal_srpm"
build_in_supported_fedora_chroots "$meta_srpm"
