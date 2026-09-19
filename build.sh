#!/usr/bin/env bash
# Install build dependencies and build the complete local Gnoblin installation.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "$0")")"

case "${1:-}" in
    '') ;;
    --yes) ;;
    --help|-h) echo "Usage: ./build.sh [--yes]"; exit 0 ;;
    *) echo "Usage: ./build.sh [--yes]" >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo 'Too many arguments' >&2; exit 2; }
privilege=()
[ "$(id -u)" -eq 0 ] || privilege=(sudo)
confirm=()

source /etc/os-release
case " $ID ${ID_LIKE:-} " in
    *' arch '*)
        [ "${1:-}" != --yes ] || confirm=(--noconfirm)
        "${privilege[@]}" pacman -Syu --needed "${confirm[@]}" \
            base-devel git just meson ninja python glib2-devel gobject-introspection \
            gnome-shell mutter gnome-session gnome-settings-daemon \
            gnome-control-center xdg-desktop-portal-gnome blueprint-compiler evolution-data-server \
            wayland-protocols egl-wayland libdisplay-info libei hyprcursor lua \
            sassc cmake intltool libxkbfile xorg-xwayland python-docutils
        ;;
    *' fedora '*)
        [ "${1:-}" != --yes ] || confirm=(-y)
        "${privilege[@]}" dnf "${confirm[@]}" install \
            dnf-plugins-core git just meson ninja-build python3 rpm-build rpmdevtools
        "${privilege[@]}" dnf "${confirm[@]}" copr enable kierandrewett/gnoblin
        "${privilege[@]}" dnf "${confirm[@]}" builddep \
            packaging/rpm/mutter.spec packaging/rpm/gnome-shell.spec \
            gnome-control-center xdg-desktop-portal-gnome
        ;;
    *) echo "Automatic dependencies are supported on Fedora and Arch/CachyOS." >&2; exit 1 ;;
esac

# A running system session can export GNOBLIN_PREFIX=/usr. Source builds always
# stay in this checkout; use the individual Just recipes for custom prefixes.
export GNOBLIN_PREFIX="$PWD/install"
export GNOBLIN_LIBDIR=lib64
# Applying our patches creates local commits; no global Git setup is needed.
export GIT_AUTHOR_NAME="${GIT_AUTHOR_NAME:-$(git config user.name || echo 'Gnoblin build')}"
export GIT_AUTHOR_EMAIL="${GIT_AUTHOR_EMAIL:-$(git config user.email || echo 'build@gnoblin.local')}"
export GIT_COMMITTER_NAME="${GIT_COMMITTER_NAME:-$GIT_AUTHOR_NAME}"
export GIT_COMMITTER_EMAIL="${GIT_COMMITTER_EMAIL:-$GIT_AUTHOR_EMAIL}"
# Validate and clear only previously generated patch state before submodule init.
for project in mutter gnome-shell gnome-control-center xdg-desktop-portal-gnome; do
    if [ -f "build/subproject-state/$project.sha256" ]; then
        just reset "$project"
    fi
done
just init
just build-local dev-settings dev-portal
printf '\nComplete Gnoblin build installed in %s\n' "$GNOBLIN_PREFIX"
