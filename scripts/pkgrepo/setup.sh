#!/bin/sh
# Set up the Gnoblin and Bingux package repositories on this system.
#
#   curl -fsSL https://repo.gnoblin.org/setup.sh | sh
#
# It shows a short list of what to set up. Gnoblin and the stable channel are selected; press Space to select Bingux or
# the nightly channel too, then Enter. It then detects your system, adds the signing key and the repositories for the package manager it finds,
# and prints the command that installs the packages. It installs no packages itself.
#
# Keys in the list: Up and Down (or k and j) move, Space selects, Enter confirms, q quits.
#
# Options, for scripts and for running without a terminal:
#   --gnoblin      set up the Gnoblin repository without asking
#   --bingux       set up the Bingux repository without asking. Bingux needs Gnoblin, so both are set up.
#   --nightly      use the nightly channel (builds of the main branch) instead of stable
#   --dry-run      print what the script would do and change nothing
#   --help         print this text
#
# GNOBLIN_REPO_URL changes the address of the repository server.
set -eu

base="${GNOBLIN_REPO_URL:-https://repo.gnoblin.org}"
channel=stable
projects=""
ask=1
dry_run=0
# The publisher writes the fingerprint of the signing key here. The script stops if the key it downloads differs.
expected_fingerprint="${GNOBLIN_KEY_FINGERPRINT:-@KEY_FINGERPRINT@}"

usage() {
    sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --gnoblin)
            projects="gnoblin $projects"
            ask=0
            ;;
        --bingux)
            projects="gnoblin bingux"
            ask=0
            ;;
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

# Colour and bold only when output goes to a terminal and NO_COLOR is not set.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    bold="$(printf '\033[1m')"
    dim="$(printf '\033[2m')"
    red="$(printf '\033[31m')"
    green="$(printf '\033[32m')"
    yellow="$(printf '\033[33m')"
    reset="$(printf '\033[0m')"
else
    bold="" dim="" red="" green="" yellow="" reset=""
fi

title() { printf '\n%s%s%s\n' "$bold" "$*" "$reset"; }
info() { printf '  %s\n' "$*"; }
ok() { printf '  %s[ok]%s %s\n' "$green" "$reset" "$*"; }
skip() { printf '  %s[--]%s %s\n' "$yellow" "$reset" "$*"; }
die() {
    printf '\n%serror:%s %s\n' "$red" "$reset" "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# Run a command as root. With --dry-run, print it instead.
as_root() {
    if [ "$dry_run" = 1 ]; then
        printf '  %s+ %s%s\n' "$dim" "$*" "$reset"
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

title "Gnoblin package setup"
info "System:  ${PRETTY_NAME:-$ID} ($arch)"

# One row of the list. Repositories are checkboxes. The channel is a choice of one, so its rows are radio buttons.
draw_row() {
    if [ "$cursor" = "$1" ]; then pointer="${bold}>${reset}"; else pointer=" "; fi
    printf '\033[2K\r  %s %s %s%s%s\n' "$pointer" "$3" "$2" "$dim" "  $4$reset" >/dev/tty
}

box() { if [ "$1" = 1 ]; then printf '[x]'; else printf '[ ]'; fi; }
radio() { if [ "$1" = "$2" ]; then printf '(*)'; else printf '( )'; fi; }

# Show the list of what to set up. The script usually runs as "curl ... | sh", so the keys come from the terminal and
# not from standard input. Without a terminal, the answer is Gnoblin only, in the stable channel.
choose_projects() {
    # /dev/tty can exist and still not open, for example in a container or a CI job, so try to open it.
    if ! (: </dev/tty >/dev/tty) 2>/dev/null; then
        projects=gnoblin
        info "No terminal, so only Gnoblin is set up. Use --bingux for Bingux and --nightly for the nightly channel."
        return 0
    fi
    pick_gnoblin=1
    pick_bingux=0
    pick_channel="$channel"
    cursor=0
    saved_tty="$(stty -g </dev/tty)"
    trap 'stty "$saved_tty" </dev/tty; printf "\n" >/dev/tty; exit 130' INT TERM
    stty -icanon -echo min 1 time 0 </dev/tty
    first=1
    while :; do
        [ "$first" = 1 ] || printf '\033[8A' >/dev/tty
        first=0
        printf '\033[2K\r\n%sRepositories%s\n' "$bold" "$reset" >/dev/tty
        draw_row 0 gnoblin "$(box "$pick_gnoblin")" "Window manager and session"
        draw_row 1 bingux "$(box "$pick_bingux")" "Desktop shell and apps. Needs Gnoblin, so it adds both"
        printf '\033[2K\r%sChannel%s\n' "$bold" "$reset" >/dev/tty
        draw_row 2 stable "$(radio stable "$pick_channel")" "Releases"
        draw_row 3 nightly "$(radio nightly "$pick_channel")" "Builds of the main branch"
        printf '\033[2K\r  %sUp/Down move   Space selects   Enter confirms   q quits%s\n' "$dim" "$reset" >/dev/tty
        key="$(dd bs=8 count=1 2>/dev/null </dev/tty)"
        case "$key" in
            "$(printf '\033[A')" | k) cursor=$(((cursor + 3) % 4)) ;;
            "$(printf '\033[B')" | j) cursor=$(((cursor + 1) % 4)) ;;
            " ")
                case "$cursor" in
                    0) pick_gnoblin=$((1 - pick_gnoblin)) ;;
                    1) pick_bingux=$((1 - pick_bingux)) ;;
                    2) pick_channel=stable ;;
                    3) pick_channel=nightly ;;
                esac
                ;;
            q | Q)
                stty "$saved_tty" </dev/tty
                printf '\n' >/dev/tty
                echo "Nothing was changed."
                exit 0
                ;;
            "") break ;;
        esac
    done
    stty "$saved_tty" </dev/tty
    trap - INT TERM
    projects=""
    [ "$pick_gnoblin" = 0 ] || projects="gnoblin"
    # Bingux runs on Gnoblin, so selecting Bingux always sets up both.
    [ "$pick_bingux" = 0 ] || projects="gnoblin bingux"
    channel="$pick_channel"
    [ -n "$projects" ] || die "nothing was selected. Nothing was changed"
}

[ "$ask" = 0 ] || choose_projects
[ -n "$projects" ] || projects=gnoblin

configured=""
key_done=0

# Download the signing key to a temporary file and print its fingerprint when gpg can read it.
fetch_key() {
    key_file="$(mktemp)"
    curl -fsSL -o "$key_file" "$base/gpg.key" || die "could not download the signing key"
    if have gpg; then
        fingerprint="$(gpg --show-keys --with-colons "$key_file" 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')"
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
    ok "signing key $fingerprint"
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
        info "$project-$channel is already set up"
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
        info "[$section] is already in /etc/pacman.conf"
    elif [ "$dry_run" = 1 ]; then
        printf '  %s+ curl -fsSL %s | tee -a /etc/pacman.conf%s\n' "$dim" "$base/$project/$channel/arch/$section.conf" "$reset"
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
        info "$project is already in /etc/apk/repositories"
    elif [ "$dry_run" = 1 ]; then
        printf '  %s+ echo %s >> /etc/apk/repositories%s\n' "$dim" "$line" "$reset"
    else
        printf '%s\n' "$line" | as_root tee -a /etc/apk/repositories >/dev/null
    fi
}

title "Setting up the $channel channel"
[ "$(id -u)" = 0 ] || [ "$dry_run" = 1 ] || info "${dim}You may be asked for your password to change the system.${reset}"

if [ "$family" = deb ] || [ "$family" = arch ]; then
    fetch_key
    check_key
fi

for project in $projects; do
    if "setup_$family" "$project"; then
        ok "$project repository added"
        configured="$configured $project"
    else
        skip "no $project packages for ${PRETTY_NAME:-this system} ($arch) in the $channel channel"
        [ "$channel" != stable ] || info "    Run the script again and select nightly to look in the nightly channel."
    fi
done

[ -n "$configured" ] || die "no repository was set up. Nothing was changed"

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
title "Done"
info "Install with:"
printf '\n      %s%s %s%s\n\n' "$bold" "$install" "$last" "$reset"
if [ "$family" = fedora ] || [ "$family" = opensuse ]; then
    info "${dim}Your package manager asks you to accept the signing key the first time.${reset}"
fi
[ -z "${key_file:-}" ] || rm -f "$key_file"
