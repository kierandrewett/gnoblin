#!/usr/bin/env bash
# Publish prepared source RPMs, with Mutter available before Shell builds.
set -euo pipefail
if [[ $# != 5 || "$1" != */* ]]; then
    echo "Usage: $0 <owner/project> <gnoblin-gsettings-desktop-schemas.src.rpm> <gnoblin-mutter.src.rpm> <gnoblin-shell.src.rpm> <gnoblin.src.rpm>" >&2
    exit 2
fi
project="$1"
schemas_srpm="$(realpath "$2")"
mutter_srpm="$(realpath "$3")"
shell_srpm="$(realpath "$4")"
meta_srpm="$(realpath "$5")"
command -v copr-cli >/dev/null
chroots=(fedora-43-x86_64 fedora-44-x86_64 fedora-45-x86_64)

build_in_supported_fedora_chroots() {
    local package="$1"
    local chroot
    local arguments=()
    for chroot in "${chroots[@]}"; do
        arguments+=(--chroot "$chroot")
    done
    copr-cli build "${arguments[@]}" "$project" "$package"
}

for package in "$schemas_srpm" "$mutter_srpm" "$shell_srpm" "$meta_srpm"; do
    [[ -f "$package" ]] || {
        echo "Missing source RPM: $package" >&2
        exit 1
    }
    [[ "$(rpm -qp --qf '%{SOURCEPACKAGE}' "$package")" == 1 ]] || {
        echo "Not a source RPM: $package" >&2
        exit 1
    }
done
[[ "$(rpm -qp --qf '%{NAME}' "$schemas_srpm")" == gnoblin-gsettings-desktop-schemas ]]
[[ "$(rpm -qp --qf '%{NAME}' "$mutter_srpm")" == gnoblin-mutter ]]
[[ "$(rpm -qp --qf '%{NAME}' "$shell_srpm")" == gnoblin-shell ]]
[[ "$(rpm -qp --qf '%{NAME}' "$meta_srpm")" == gnoblin ]]
# copr-cli waits by default. Build each dependency in all supported Fedora
# chroots before starting its dependent package; any failed chroot stops here.
build_in_supported_fedora_chroots "$schemas_srpm"
build_in_supported_fedora_chroots "$mutter_srpm"
build_in_supported_fedora_chroots "$shell_srpm"
build_in_supported_fedora_chroots "$meta_srpm"
