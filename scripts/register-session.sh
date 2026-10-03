#!/usr/bin/env bash
# Register a source build with the login manager and systemd --user.
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

required=("$DESKTOP" "$PREFIX/bin/gnoblin" "$PREFIX/libexec/gnoblin-env.sh"
    "$STANDALONE_TARGET" "$IDLE_SERVICE" "$IDLE_BINARY")
for f in "${required[@]}"; do
    [ -f "$f" ] || {
        echo "Missing $f -- run ./build.sh first" >&2
        exit 1
    }
done

portal_files=(
    "$PORTAL_UNIT"
    "$PORTAL_BINARY"
    "$PORTAL_DESCRIPTOR"
    "$PORTAL_CONFIGURATION"
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

command -v systemctl >/dev/null 2>&1 || {
    echo "systemctl not found -- this needs a systemd user session" >&2
    exit 1
}
command -v python3 >/dev/null 2>&1 || {
    echo 'Missing python3; it is required to create the standalone login entry.' >&2
    exit 1
}
command -v dbus-update-activation-environment >/dev/null 2>&1 || {
    echo "Missing dbus-update-activation-environment; install your distribution's D-Bus tools." >&2
    exit 1
}
systemctl --user show-environment >/dev/null 2>&1 || {
    echo 'The systemd user manager is unavailable. Register from a logged-in systemd session.' >&2
    exit 1
}
if "$with_portal" && ! systemctl --user cat xdg-desktop-portal.service >/dev/null 2>&1; then
    echo 'Missing user service xdg-desktop-portal.service. Install the session runtime packages in docs/install-source.md.' >&2
    exit 1
fi
# A previous source tarball may have registered the same unit names from a
# different prefix. Refresh only links that point to Gnoblin's own unit paths.
linked_units=(gnoblin-session.target gnoblin-idle.service)
if "$with_portal"; then
    linked_units+=(xdg-desktop-portal-gnoblin.service)
fi
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

command -v sudo >/dev/null 2>&1 || {
    echo 'sudo is needed to add the login screen entries.' >&2
    exit 1
}
sudo -v

printf '%s==>%s Registering Gnoblin with systemd and the login screen\n' "$blue" "$reset"
unit_files=("$STANDALONE_TARGET" "$IDLE_SERVICE")
if "$with_portal"; then
    unit_files+=("$PORTAL_UNIT")
fi
systemctl --user --force link "${unit_files[@]}"
desktop_to_install="$(mktemp)"
trap 'rm -f -- "$desktop_to_install"' EXIT
python3 - "$DESKTOP" "$desktop_to_install" <<'PY'
from pathlib import Path
import sys

source, destination = map(Path, sys.argv[1:])
lines = []
for line in source.read_text().splitlines():
    if line.startswith('DesktopNames='):
        line = 'DesktopNames=Gnoblin;'
    lines.append(line)
destination.write_text('\n'.join(lines) + '\n')
PY
systemctl --user daemon-reload
sudo install -Dm644 "$desktop_to_install" /usr/share/wayland-sessions/gnoblin.desktop
printf '%sGnoblin is available%s at login. Choose the existing GNOME session to switch back to GNOME.\n' "$green" "$reset"
if "$with_portal"; then
    sudo install -Dm644 "$PORTAL_DESCRIPTOR" /usr/share/xdg-desktop-portal/portals/gnoblin.portal
    sudo install -Dm644 "$PORTAL_CONFIGURATION" /usr/share/xdg-desktop-portal/gnoblin-portals.conf
    sudo install -Dm644 "$PORTAL_DBUS" /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
else
    echo 'No Gnoblin portal backend in this build; using your installed backend.'
    echo 'To choose another, set gnoblin.configure.portals in your Lua config.'
fi
