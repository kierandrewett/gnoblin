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

PREFIX="${1:?usage: register-session.sh <prefix> [--gnome-session]}"
case "${2:-}" in
    '') standalone=true ;;
    --standalone) standalone=true ;;
    --gnome-session) standalone=false ;;
    *)
        echo 'Usage: register-session.sh <prefix> [--gnome-session]' >&2
        exit 2
        ;;
esac
if [ "$#" -gt 2 ]; then
    echo 'Usage: register-session.sh <prefix> [--gnome-session]' >&2
    exit 2
fi
if [ ! -d "$PREFIX" ]; then
    echo "Gnoblin prefix does not exist: $PREFIX" >&2
    echo 'Build the local session first: ./build.sh' >&2
    exit 1
fi
PREFIX="$(cd "$PREFIX" && pwd)"
source "$(dirname "$0")/../src/tools/gnoblin-env.sh"
gnoblin_env_validate_install_prefix "$PREFIX" || exit
UNIT_DIR="$PREFIX/lib/systemd/user"
TARGET="$UNIT_DIR/org.gnoblin.Shell.target"
STANDALONE_TARGET="$UNIT_DIR/gnoblin-session.target"
IDLE_SERVICE="$UNIT_DIR/gnoblin-idle.service"
IDLE_BINARY="$PREFIX/libexec/gnoblin-idle"
SERVICE="$UNIT_DIR/org.gnoblin.Shell@wayland.service"
DESKTOP="$PREFIX/share/wayland-sessions/gnoblin.desktop"
SESSION="$PREFIX/share/gnome-session/sessions/gnoblin.session"
DROPIN="$UNIT_DIR/gnome-session@gnoblin.target.d/gnoblin.conf"
PORTAL_UNIT="$UNIT_DIR/xdg-desktop-portal-gnoblin.service"
PORTAL_BINARY="$PREFIX/libexec/xdg-desktop-portal-gnoblin"
PORTAL_DESCRIPTOR="$PREFIX/share/xdg-desktop-portal/portals/gnoblin.portal"
PORTAL_CONFIG="$PREFIX/share/xdg-desktop-portal/gnoblin-portals.conf"
PORTAL_DBUS="$PREFIX/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service"
USER_DROPIN="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
USER_UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

required=("$DESKTOP" "$PORTAL_UNIT" "$PORTAL_BINARY" "$PORTAL_DESCRIPTOR" "$PORTAL_CONFIG" "$PORTAL_DBUS"
    "$PREFIX/bin/gnoblin" "$PREFIX/libexec/gnoblin-env.sh")
if "$standalone"; then
    required+=("$STANDALONE_TARGET" "$IDLE_SERVICE" "$IDLE_BINARY"
        "$PREFIX/bin/gnoblin-shell-service")
else
    required+=("$TARGET" "$SERVICE" "$SESSION" "$DROPIN")
fi
for f in "${required[@]}"; do
    [ -f "$f" ] || {
        echo "Missing $f -- run ./build.sh first" >&2
        exit 1
    }
done

command -v systemctl >/dev/null 2>&1 || {
    echo "systemctl not found -- this needs a systemd user session" >&2
    exit 1
}
if ! "$standalone"; then
    command -v gnome-session >/dev/null 2>&1 || {
        echo 'Missing gnome-session. Install it before registering Gnoblin. See docs/install-source.md.' >&2
        exit 1
    }
fi
if "$standalone"; then
    command -v python3 >/dev/null 2>&1 || {
        echo 'Missing python3; it is required to create the standalone login entry.' >&2
        exit 1
    }
    command -v dbus-update-activation-environment >/dev/null 2>&1 || {
        echo "Missing dbus-update-activation-environment; install your distribution's D-Bus tools." >&2
        exit 1
    }
fi
systemctl --user show-environment >/dev/null 2>&1 || {
    echo 'The systemd user manager is unavailable. Register from a logged-in systemd session.' >&2
    exit 1
}
services=(xdg-desktop-portal.service)
if ! "$standalone"; then
    services+=(org.gnome.SettingsDaemon.Power.target)
fi
for unit in "${services[@]}"; do
    systemctl --user cat "$unit" >/dev/null 2>&1 || {
        echo "Missing user service $unit. Install the session runtime packages in docs/install-source.md." >&2
        exit 1
    }
done
if ! "$standalone" && [ -e "$USER_DROPIN" ] && [ ! -L "$USER_DROPIN" ]; then
    echo "Existing custom drop-in at $USER_DROPIN; move it aside before registering Gnoblin." >&2
    exit 1
fi
# A previous source tarball may have registered the same unit names from a
# different prefix. Refresh only links that point to Gnoblin's own unit paths.
linked_units=(xdg-desktop-portal-gnoblin.service)
if "$standalone"; then
    linked_units+=(gnoblin-session.target gnoblin-idle.service)
else
    linked_units+=(org.gnoblin.Shell.target org.gnoblin.Shell@wayland.service)
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
desktop_to_install="$DESKTOP"
if "$standalone"; then
    systemctl --user --force link "$STANDALONE_TARGET" "$IDLE_SERVICE" "$PORTAL_UNIT"
    desktop_to_install="$(mktemp)"
    trap 'rm -f -- "$desktop_to_install"' EXIT
    python3 - "$DESKTOP" "$desktop_to_install" <<'PY'
from pathlib import Path
import sys

source, destination = map(Path, sys.argv[1:])
lines = []
for line in source.read_text().splitlines():
    if line.startswith('Exec='):
        line = 'Exec=env GNOBLIN_STANDALONE_SESSION=1 ' + line[5:]
    elif line.startswith('DesktopNames='):
        line = 'DesktopNames=Gnoblin;'
    lines.append(line)
destination.write_text('\n'.join(lines) + '\n')
PY
else
    systemctl --user --force link "$TARGET" "$SERVICE" "$PORTAL_UNIT"
    mkdir -p "$(dirname "$USER_DROPIN")"
    ln -sfn "$DROPIN" "$USER_DROPIN"
fi
systemctl --user daemon-reload
sudo install -Dm644 "$desktop_to_install" /usr/share/wayland-sessions/gnoblin.desktop
if ! "$standalone"; then
    sudo install -Dm644 "$SESSION" /usr/share/gnome-session/sessions/gnoblin.session
fi
sudo install -Dm644 "$PORTAL_DESCRIPTOR" /usr/share/xdg-desktop-portal/portals/gnoblin.portal
sudo install -Dm644 "$PORTAL_CONFIG" /usr/share/xdg-desktop-portal/gnoblin-portals.conf
sudo install -Dm644 "$PORTAL_DBUS" /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
if "$standalone"; then
    printf '%sGnoblin standalone is available%s at login. Settings-daemon services are not started.\n' "$green" "$reset"
else
    printf '%sGnoblin is available%s in the login screen session selector. See docs/install-source.md to remove it.\n' "$green" "$reset"
fi
