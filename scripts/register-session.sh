#!/usr/bin/env bash
# Register a source build with the login manager. User-systemd integration is
# optional and is linked only when an active user manager is available.
# build.sh is the public entry point. This script changes state outside the
# build prefix, so it runs only for an explicit registration request.
set -euo pipefail

blue='' green='' reset=''
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    blue=$'\033[1;36m'
    green=$'\033[1;32m'
    reset=$'\033[0m'
fi

if [ "$#" -ne 1 ]; then
    echo 'Usage: register-session.sh <prefix>' >&2
    exit 2
fi
PREFIX="$1"
if [ ! -d "$PREFIX" ]; then
    echo "Gnoblin prefix does not exist: $PREFIX" >&2
    echo 'Build the local session first: ./build.sh' >&2
    exit 1
fi
PREFIX="$(cd "$PREFIX" && pwd)"
source "$(dirname "$0")/../src/tools/gnoblin-env.sh"
gnoblin_env_validate_install_prefix "$PREFIX" || exit
UNIT_DIR="$PREFIX/lib/systemd/user"
STANDALONE_TARGET="$UNIT_DIR/gnoblin-session.target"
IDLE_SERVICE="$UNIT_DIR/gnoblin-idle.service"
IDLE_BINARY="$PREFIX/libexec/gnoblin-idle"
DESKTOP="$PREFIX/share/wayland-sessions/gnoblin.desktop"
PORTAL_UNIT="$UNIT_DIR/xdg-desktop-portal-gnoblin.service"
PORTAL_BINARY="$PREFIX/libexec/xdg-desktop-portal-gnoblin"
PORTAL_DESCRIPTOR="$PREFIX/share/xdg-desktop-portal/portals/gnoblin.portal"
PORTAL_CONFIGURATION="$PREFIX/share/xdg-desktop-portal/gnoblin-portals.conf"
PORTAL_DBUS="$PREFIX/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service"
USER_UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
USER_BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
GNOBLINCTL_LINK="$USER_BIN_DIR/gnoblinctl"
# man-db finds user manual pages by mapping each directory on PATH to the share/man beside it, not from XDG_DATA_HOME. So
# the pages go next to the bin directory that holds the gnoblinctl link, which is the directory the user has on PATH.
USER_MAN_DIR="$(dirname "$USER_BIN_DIR")/share/man/man1"
LEGACY_RECOVERY_LINK="$USER_UNIT_DIR/gnoblin-recovery.service"

required=("$DESKTOP" "$PREFIX/bin/gnoblin" "$PREFIX/bin/gnoblinctl"
    "$PREFIX/libexec/gnoblin-env.sh"
    "$STANDALONE_TARGET" "$IDLE_SERVICE" "$IDLE_BINARY" "$PORTAL_CONFIGURATION")
for f in "${required[@]}"; do
    [ -f "$f" ] || {
        echo "Missing $f -- run ./build.sh first" >&2
        exit 1
    }
done
executables=("$PREFIX/bin/gnoblin" "$PREFIX/bin/gnoblinctl" "$IDLE_BINARY")
for f in "${executables[@]}"; do
    [ -x "$f" ] || {
        echo "Not executable: $f -- rebuild with ./build.sh" >&2
        exit 1
    }
done

# GDM starts this binary directly, before Gnoblin can set its runtime paths.
# A build-tree copy may exist and be executable while its ELF loader paths
# still point outside the installed prefix. Reject that registration early.
if ! env -u LD_LIBRARY_PATH -u LD_PRELOAD "$PREFIX/bin/gnoblin" --version >/dev/null; then
    echo 'The installed Gnoblin executable cannot load its runtime libraries.' >&2
    echo 'Run ./build.sh to install the built artifacts through Meson, then register again.' >&2
    exit 1
fi

portal_files=(
    "$PORTAL_UNIT"
    "$PORTAL_BINARY"
    "$PORTAL_DESCRIPTOR"
    "$PORTAL_DBUS"
)
portal_count=0
for f in "${portal_files[@]}"; do
    [ -f "$f" ] && ((portal_count += 1))
done
if ((portal_count != 0 && portal_count != ${#portal_files[@]})); then
    echo 'The Gnoblin portal backend is only partly installed in this prefix.' >&2
    echo 'Rebuild it with ./build.sh --with-portal or remove the incomplete portal files.' >&2
    exit 1
fi
with_portal=false
((portal_count == ${#portal_files[@]})) && with_portal=true

command -v python3 >/dev/null 2>&1 || {
    echo 'Missing python3; it is required to create the standalone login entry.' >&2
    exit 1
}
have_user_systemd=false
if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
    have_user_systemd=true
fi
if "$with_portal" && "$have_user_systemd" && ! systemctl --user cat xdg-desktop-portal.service >/dev/null 2>&1; then
    echo 'Missing user service xdg-desktop-portal.service. Install the session runtime packages in docs/install-source.md.' >&2
    exit 1
fi
# A previous source tarball may have registered the same unit names from a
# different prefix. Refresh only links that point to Gnoblin's own unit paths.
# The build writes the list of public files. Register what it built, not a list kept in this script.
PUBLIC_ENTRIES="$PREFIX/share/gnoblin/public-entries.txt"
[ -f "$PUBLIC_ENTRIES" ] || {
    echo "Missing $PUBLIC_ENTRIES -- run make first" >&2
    exit 1
}
mapfile -t public_entries <"$PUBLIC_ENTRIES"
unit_files=()
linked_units=()
for entry in "${public_entries[@]}"; do
    case "$entry" in
        lib/systemd/user/*)
            unit_files+=("$PREFIX/$entry")
            linked_units+=("${entry##*/}")
            ;;
    esac
done
if "$have_user_systemd"; then
    for unit in "${linked_units[@]}"; do
        link="$USER_UNIT_DIR/$unit"
        if [ -e "$link" ] && [ ! -L "$link" ]; then
            echo "Existing custom unit at $link; move it aside before registering Gnoblin." >&2
            exit 1
        fi
        if [ -L "$link" ]; then
            case "$(readlink "$link")" in
                */lib/systemd/user/"$unit") ;;
                *)
                    echo "Existing custom unit link at $link; move it aside before registering Gnoblin." >&2
                    exit 1
                    ;;
            esac
        fi
    done
fi

# A prior source registration linked this compositor-external recovery unit.
# Remove only that known link; a user-owned unit remains untouched.
if [ -L "$LEGACY_RECOVERY_LINK" ] &&
    [ "$(readlink "$LEGACY_RECOVERY_LINK")" = "$PREFIX/lib/systemd/user/gnoblin-recovery.service" ]; then
    rm "$LEGACY_RECOVERY_LINK"
fi

if [ -L "$GNOBLINCTL_LINK" ]; then
    existing_target="$(readlink "$GNOBLINCTL_LINK")"
    if [ "$existing_target" != "$PREFIX/bin/gnoblinctl" ]; then
        echo "Existing gnoblinctl link points elsewhere: $GNOBLINCTL_LINK -> $existing_target" >&2
        echo 'Move it aside before registering this Gnoblin prefix.' >&2
        exit 1
    fi
elif [ -e "$GNOBLINCTL_LINK" ]; then
    echo "Existing command at $GNOBLINCTL_LINK; move it aside before registering Gnoblin." >&2
    exit 1
fi

command -v sudo >/dev/null 2>&1 || {
    echo 'sudo is needed to add the login screen entries.' >&2
    exit 1
}
sudo -v

printf '%s==>%s Registering Gnoblin with the login screen\n\n' "$blue" "$reset"

# Print one row for every file or link this script makes. A link shows its target. Rows carry no category,
# so a new file needs no change here.
installed() {
    if [ -L "$1" ]; then
        printf '  %s -> %s\n' "$1" "$(readlink "$1")"
    else
        printf '  %s\n' "$1"
    fi
}

package_owns() {
    { command -v rpm >/dev/null 2>&1 && rpm -qf "$1" >/dev/null 2>&1; } ||
        { command -v pacman >/dev/null 2>&1 && pacman -Qo "$1" >/dev/null 2>&1; }
}

install_system_file() {
    sudo install -Dm644 "$1" "$2"
    installed "$2"
}

if "$have_user_systemd"; then
    systemctl --user --force link "${unit_files[@]}" >/dev/null
    for unit in "${unit_files[@]}"; do
        installed "$USER_UNIT_DIR/$(basename "$unit")"
    done
    systemctl --user daemon-reload
else
    echo 'No systemd user manager detected; Gnoblin core will run without user units.'
fi

desktop_to_install="$(mktemp)"
trap 'rm -f -- "$desktop_to_install"' EXIT
for entry in "${public_entries[@]}"; do
    case "$entry" in
        share/wayland-sessions/*.desktop)
            # GDM must not wait for this session to register with GNOME Session.
            python3 - "$PREFIX/$entry" "$desktop_to_install" <<'PY'
from pathlib import Path
import sys

source, destination = map(Path, sys.argv[1:])
lines = []
for line in source.read_text().splitlines():
    if line.startswith('DesktopNames='):
        line = 'DesktopNames=Gnoblin;'
    elif line.startswith('X-GDM-SessionRegisters='):
        line = 'X-GDM-SessionRegisters=false'
    lines.append(line)
destination.write_text('\n'.join(lines) + '\n')
PY
            install_system_file "$desktop_to_install" "/usr/$entry"
            ;;
        share/xdg-desktop-portal/* | share/dbus-1/services/*)
            install_system_file "$PREFIX/$entry" "/usr/$entry"
            ;;
        share/man/man1/*)
            # A package that ships the same page owns it. Leave that file alone.
            if package_owns "/usr/$entry"; then
                echo "  Left /usr/$entry alone: an installed package owns it." >&2
            else
                install_system_file "$PREFIX/$entry" "/usr/$entry"
            fi
            ;;
    esac
done

# Make the prefix-built native CLI discoverable. Preserve any existing command
# rather than replacing a user's script or a CLI from another installation.
mkdir -p "$USER_BIN_DIR"
if [ ! -L "$GNOBLINCTL_LINK" ]; then
    ln -s "$PREFIX/bin/gnoblinctl" "$GNOBLINCTL_LINK"
fi
installed "$GNOBLINCTL_LINK"

# man searches ~/.local/share/man, and a source build keeps its pages inside the prefix. Link them so that
# "man gnoblin" and "man gnoblinctl" work. A rebuild rewrites the pages in place behind the link. Create the link where
# nothing exists, and refresh only a link that already points into a Gnoblin prefix. Anything else is the user's, so it
# stays and the script says so.
mkdir -p "$USER_MAN_DIR"
for entry in "${public_entries[@]}"; do
    case "$entry" in
        share/man/man1/*) ;;
        *) continue ;;
    esac
    page="${entry##*/}"
    source_page="$PREFIX/$entry"
    link="$USER_MAN_DIR/$page"
    if [ ! -e "$link" ] && [ ! -L "$link" ]; then
        ln -s "$source_page" "$link"
        installed "$link"
    elif [ -L "$link" ]; then
        target="$(readlink "$link")"
        linked_prefix="${target%/share/man/man1/$page}"
        if [ "$linked_prefix" != "$target" ] && [ -x "$linked_prefix/bin/gnoblinctl" ]; then
            ln -sfn "$source_page" "$link"
            installed "$link"
        else
            echo "  Left $link alone: it points to $target, which is not a Gnoblin prefix." >&2
        fi
    else
        echo "  Left $link alone: it is not a link." >&2
    fi
done

printf '\n%sGnoblin is available%s at login.\n' "$green" "$reset"
if ! "$with_portal"; then
    echo 'Gnoblin portal backend not built; the default route will select another installed backend.'
    echo 'Set gnoblin.configure.portals in Lua to choose a specific backend.'
fi
