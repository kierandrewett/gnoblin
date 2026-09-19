#!/usr/bin/env bash
# Publish prepared source RPMs, with Mutter available before Shell builds.
set -euo pipefail
if [[ $# != 4 || "$1" != */* ]]; then
    echo "Usage: $0 <owner/project> <gnoblin-mutter.src.rpm> <gnoblin-shell.src.rpm> <gnoblin.src.rpm>" >&2
    exit 2
fi
project="$1"
mutter_srpm="$(realpath "$2")"
shell_srpm="$(realpath "$3")"
meta_srpm="$(realpath "$4")"
command -v copr-cli >/dev/null
for package in "$mutter_srpm" "$shell_srpm" "$meta_srpm"; do
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
[[ "$(rpm -qp --qf '%{NAME}' "$shell_srpm")" == gnoblin-shell ]]
[[ "$(rpm -qp --qf '%{NAME}' "$meta_srpm")" == gnoblin ]]
# copr-cli waits by default. A failed Mutter build stops publication here.
copr-cli build "$project" "$mutter_srpm"
copr-cli build "$project" "$shell_srpm"
copr-cli build "$project" "$meta_srpm"
