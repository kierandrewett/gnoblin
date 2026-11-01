#!/bin/sh
# Set up the Gnoblin and Bingux package repositories on this system.
#
#   curl -fsSL https://repo.gnoblin.org/setup.sh | sh
#   curl -fsSL https://repo.gnoblin.org/setup.sh | sh -s -- --bingux --nightly
#
# It detects your system, adds the signing key and the repository for the package manager it finds, and prints the
# command that installs the packages. It installs no packages itself.
#
# Options:
#   --bingux       also set up the Bingux repository. Bingux needs Gnoblin, so both are set up.
#   --nightly      use the nightly channel (the newest build of the main branch) instead of stable
#   --dry-run      print what the script would do and change nothing
#   --help         print this text
#
# GNOBLIN_REPO_URL changes the address of the repository server.
set -eu

base="${GNOBLIN_REPO_URL:-https://repo.gnoblin.org}"
channel=stable
projects=gnoblin
dry_run=0
# The publisher writes the fingerprint of the signing key here. The script stops if the key it downloads differs.
expected_fingerprint="${GNOBLIN_KEY_FINGERPRINT:-@KEY_FINGERPRINT@}"

usage() {
    sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bingux) projects="gnoblin bingux" ;;
        --nightly) channel=nightly ;;
        --dry-run) dry_run=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "setup: unknown option $1 (see --help)" >&2
            exit 2
            ;;
    esac
    shift
done

say() { printf 'setup: %s\n' "$*"; }
die() {
    printf 'setup: %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# Run a command as root. With --dry-run, print it instead.
as_root() {
    if [ "$dry_run" = 1 ]; then
        printf '+ %s\n' "$*"
    elif [ "$(id -u)" = 0 ]; then
        "$@"
    elif have sudo; then
        sudo "$@"
    elif have doas; then
        doas "$@"
    else
        die "run this script as root, or install sudo"
    fi
}

# Does this address exist? A check of the server, so the script never adds a repository that has no packages for this
# system.
exists() { curl -fsI -o /dev/null "$1" 2>/dev/null; }

have curl || die "curl is needed"
[ -r /etc/os-release ] || die "cannot read /etc/os-release"
# shellcheck disable=SC1091
. /etc/os-release

arch="$(uname -m)"
family=""
for id in ${ID:-} ${ID_LIKE:-}; do
    case "$id" in
        fedora | rhel | centos) family=fedora ;;
        opensuse* | suse | sles) family=opensuse ;;
        debian | ubuntu) family=deb ;;
        arch) family=arch ;;
        alpine) family=alpine ;;
    esac
    [ -n "$family" ] && break
done
[ -n "$family" ] || die "this system (${PRETTY_NAME:-unknown}) is not one the repositories cover. See https://gnoblin.org/install-source to build from source"

configured=""
key_done=0

# Download the signing key to a temporary file and print its fingerprint when gpg can read it.
fetch_key() {
    key_file="$(mktemp)"
    curl -fsSL -o "$key_file" "$base/gpg.key" || die "could not download the signing key"
    if have gpg; then
        fingerprint="$(gpg --show-keys --with-colons "$key_file" 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')"
        [ -z "$fingerprint" ] || say "signing key fingerprint: $fingerprint"
    fi
}

# Stop unless the downloaded key is the one named in this script. Debian, Ubuntu and Arch Linux trust the key without
# asking, so this check is the only one.
check_key() {
    [ "$dry_run" = 1 ] && return 0
    case "$expected_fingerprint" in
        *@*) die "this is the template of the script. Use the copy at $base/setup.sh" ;;
    esac
    [ -n "${fingerprint:-}" ] || die "gpg is needed to check the signing key"
    [ "$fingerprint" = "$expected_fingerprint" ] || die "the signing key does not match the expected fingerprint. Nothing was changed"
}

setup_fedora() {
    project="$1"
    exists "$base/$project/$channel/fedora/$VERSION_ID/$arch/repodata/repomd.xml" || return 1
    as_root curl -fsSL -o "/etc/yum.repos.d/$project-$channel.repo" "$base/$project/$channel/fedora/$project-$channel.repo"
}

setup_opensuse() {
    project="$1"
    exists "$base/$project/$channel/opensuse/tumbleweed/$arch/repodata/repomd.xml" || return 1
    if [ -e "/etc/zypp/repos.d/$project-$channel.repo" ]; then
        say "the $project-$channel repository is already set up"
    else
        as_root zypper --non-interactive addrepo --refresh \
            "$base/$project/$channel/opensuse/tumbleweed/$project-$channel.repo"
    fi
}

setup_deb() {
    project="$1"
    codename="${VERSION_CODENAME:-${UBUNTU_CODENAME:-}}"
    [ -n "$codename" ] || return 1
    exists "$base/$project/$channel/deb/$project-$channel-$codename.sources" || return 1
    as_root install -d -m 0755 /etc/apt/keyrings
    as_root curl -fsSL -o "/etc/apt/keyrings/$project.asc" "$base/gpg.key"
    as_root curl -fsSL -o "/etc/apt/sources.list.d/$project-$channel.sources" \
        "$base/$project/$channel/deb/$project-$channel-$codename.sources"
}

setup_arch() {
    project="$1"
    if [ "$channel" = stable ]; then section="$project"; else section="$project-$channel"; fi
    exists "$base/$project/$channel/arch/$arch/$section.db" || return 1
    if [ "$key_done" = 0 ]; then
        have gpg || die "gpg is needed to trust the signing key"
        as_root pacman-key --add "$key_file"
        as_root pacman-key --lsign-key "$fingerprint"
        key_done=1
    fi
    if grep -q "^\[$section\]" /etc/pacman.conf 2>/dev/null; then
        say "the [$section] repository is already in /etc/pacman.conf"
    elif [ "$dry_run" = 1 ]; then
        printf '+ curl -fsSL %s | tee -a /etc/pacman.conf\n' "$base/$project/$channel/arch/$section.conf"
    else
        curl -fsSL "$base/$project/$channel/arch/$section.conf" | as_root tee -a /etc/pacman.conf >/dev/null
    fi
}

setup_alpine() {
    project="$1"
    apk_arch="$(apk --print-arch 2>/dev/null || echo "$arch")"
    line="$base/$project/$channel/alpine/edge"
    exists "$line/$apk_arch/APKINDEX.tar.gz" || return 1
    as_root curl -fsSL -o "/etc/apk/keys/$project.rsa.pub" "$base/$project.rsa.pub"
    if grep -qxF "$line" /etc/apk/repositories 2>/dev/null; then
        say "the $project repository is already in /etc/apk/repositories"
    elif [ "$dry_run" = 1 ]; then
        printf '+ echo %s >> /etc/apk/repositories\n' "$line"
    else
        printf '%s\n' "$line" | as_root tee -a /etc/apk/repositories >/dev/null
    fi
}

if [ "$family" = deb ] || [ "$family" = arch ]; then
    fetch_key
    check_key
fi

for project in $projects; do
    if "setup_$family" "$project"; then
        say "set up the $project $channel repository"
        configured="$configured $project"
    else
        say "no $project packages for ${PRETTY_NAME:-this system} on $arch in the $channel channel, skipped"
        [ "$channel" != stable ] || say "to look in the nightly channel, run the script again with --nightly"
    fi
done

[ -n "$configured" ] || die "no repository was set up"

case "$family" in
    deb) as_root apt-get update ;;
    arch) as_root pacman -Sy ;;
    alpine) as_root apk update ;;
esac

case "$family" in
    fedora) install="sudo dnf install" ;;
    opensuse) install="sudo zypper install" ;;
    deb) install="sudo apt install" ;;
    arch) install="sudo pacman -S" ;;
    alpine) install="apk add" ;;
esac

last=""
for project in $configured; do last="$project"; done
say "done. Install with: $install $last"
[ "$family" = fedora ] || [ "$family" = opensuse ] && say "your package manager asks you to accept the signing key the first time"
[ -z "${key_file:-}" ] || rm -f "$key_file"
