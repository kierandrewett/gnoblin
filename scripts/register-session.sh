#!/usr/bin/env bash
# Install a finished build for good and register it with the login manager.
#
# usage: register-session.sh PREFIX [STAGE_ROOT]
#
#   PREFIX      where the runtime lives on this machine, for example /usr/local/lib/gnoblin. The binaries have this path
#               compiled in, so the build must have been made for it.
#   STAGE_ROOT  where the build wrote its files, like DESTDIR (for example build/stage). The script copies
#               STAGE_ROOT/PREFIX to PREFIX. Leave it out when PREFIX already holds the runtime.
#
# Nothing here depends on the source directory afterwards. The runtime, the login entry, the portal files, the systemd
# user units, the commands and the man pages are all copies below /usr and /usr/local.
# build.sh and make are the public entry points. This script changes state outside the build tree, so it runs only for
# an explicit install request.
set -euo pipefail

blue='' green='' reset=''
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    blue=$'\033[1;36m'
    green=$'\033[1;32m'
    reset=$'\033[0m'
fi

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo 'Usage: register-session.sh <prefix> [<stage root>]' >&2
    exit 2
fi
PREFIX="${1%/}"
STAGE="${2:-}"
STAGE="${STAGE%/}"
case "$PREFIX" in
    /*) ;;
    *)
        echo "The prefix must be an absolute path: $PREFIX" >&2
        exit 2
        ;;
esac
SRC="$STAGE$PREFIX"
if [ ! -d "$SRC" ]; then
    echo "Gnoblin build does not exist: $SRC" >&2
    echo 'Build it first: make' >&2
    exit 1
fi
source "$(dirname "$0")/../src/tools/gnoblin-env.sh"
gnoblin_env_validate_install_prefix "$PREFIX" || exit

BIN_DIR="${GNOBLIN_BIN_DIR:-/usr/local/bin}"
SYSTEM_ROOT="${GNOBLIN_SYSTEM_ROOT:-/usr}"
USER_UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
USER_BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
USER_MAN_DIR="$(dirname "$USER_BIN_DIR")/share/man/man1"
PUBLIC_ENTRIES="$SRC/share/gnoblin/public-entries.txt"

required=("$SRC/bin/gnoblin" "$SRC/bin/gnoblinctl" "$SRC/libexec/gnoblin-idle" "$SRC/libexec/gnoblin-env.sh"
    "$SRC/share/xdg-desktop-portal/gnoblin-portals.conf" "$SRC/share/wayland-sessions/gnoblin.desktop"
    "$SRC/lib/systemd/user/gnoblin-session.target" "$SRC/lib/systemd/user/gnoblin-idle.service" "$PUBLIC_ENTRIES")
for f in "${required[@]}"; do
    [ -f "$f" ] || {
        echo "Missing $f -- run make first" >&2
        exit 1
    }
done
for f in "$SRC/bin/gnoblin" "$SRC/bin/gnoblinctl" "$SRC/libexec/gnoblin-idle"; do
    [ -x "$f" ] || {
        echo "Not executable: $f -- rebuild with make" >&2
        exit 1
    }
done

# GDM starts this binary directly, before Gnoblin can set its runtime paths. The loader must find every library without
# a developer's LD_LIBRARY_PATH. Mutter's libraries search an absolute path below the prefix, so a staged copy is checked
# with the stage visible at the prefix.
loader_check=(env -u LD_LIBRARY_PATH -u LD_PRELOAD "$PREFIX/bin/gnoblin" --version)
if [ -n "$STAGE" ]; then
    loader_check=("$(dirname "$0")/run-staged.sh" "$PREFIX" "$STAGE" "${loader_check[@]}")
fi
if ! "${loader_check[@]}" >/dev/null; then
    echo 'The Gnoblin executable cannot load its runtime libraries.' >&2
    echo 'Run make to install the built artifacts through Meson, then install again.' >&2
    exit 1
fi

# The portal backend is optional, but it must be whole.
portal_files=("$SRC/lib/systemd/user/xdg-desktop-portal-gnoblin.service" "$SRC/libexec/xdg-desktop-portal-gnoblin"
    "$SRC/share/xdg-desktop-portal/portals/gnoblin.portal"
    "$SRC/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service")
portal_count=0
for f in "${portal_files[@]}"; do
    [ -f "$f" ] && ((portal_count += 1))
done
if ((portal_count != 0 && portal_count != ${#portal_files[@]})); then
    echo 'The Gnoblin portal backend is only partly built.' >&2
    echo 'Rebuild it with make, or remove the incomplete portal files.' >&2
    exit 1
fi
with_portal=false
((portal_count == ${#portal_files[@]})) && with_portal=true

command -v python3 >/dev/null 2>&1 || {
    echo 'Missing python3; it is required to create the login entry.' >&2
    exit 1
}
command -v sudo >/dev/null 2>&1 || {
    echo 'sudo is needed to install the runtime and the login screen entries.' >&2
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
mapfile -t public_entries <"$PUBLIC_ENTRIES"

# Print the name of the installed package that owns a path, or nothing.
package_owner() {
    if command -v rpm >/dev/null 2>&1; then
        rpm -qf --qf '%{NAME}\n' "$1" 2>/dev/null | head -n 1 | grep -v 'not owned' || true
    elif command -v pacman >/dev/null 2>&1; then
        pacman -Qoq "$1" 2>/dev/null | head -n 1 || true
    fi
}

# Stop before any change if a package already ships one of these files. A half install would mix two builds: the
# package's login entry and units would start the package's runtime, not this one.
conflicts=0
owners=()
for entry in "${public_entries[@]}"; do
    owner="$(package_owner "$SYSTEM_ROOT/$entry")"
    if [ -n "$owner" ]; then
        echo "$SYSTEM_ROOT/$entry belongs to the installed package $owner." >&2
        conflicts=$((conflicts + 1))
        [[ " ${owners[*]:-} " == *" $owner "* ]] || owners+=("$owner")
    fi
done
if ((conflicts > 0)); then
    echo 'Nothing was changed. Remove the package, then run make install again:' >&2
    echo "  sudo dnf remove ${owners[*]}    (or the package manager of this system)" >&2
    exit 1
fi

sudo -v
printf '%s==>%s Installing Gnoblin\n\n' "$blue" "$reset"

# Print one row for every file or link this script makes. A link shows its target. Rows carry no category,
# so a new file needs no change here.
installed() {
    if [ -L "$1" ]; then
        printf '  %s -> %s\n' "$1" "$(readlink "$1")"
    else
        printf '  %s\n' "$1"
    fi
}

install_public_file() {
    sudo install -Dm644 "$1" "$2"
    installed "$2"
}

# The runtime. Files belong to root, so nobody can change what the login manager runs. Existing directories keep their
# permissions.
if [ -n "$STAGE" ]; then
    sudo mkdir -p "$PREFIX"
    tar -C "$STAGE$PREFIX" --owner=0 --group=0 -cf - . | sudo tar -C "$PREFIX" --no-overwrite-dir -xpf -
    printf '  %s (%s files)\n' "$PREFIX" "$(find "$SRC" \( -type f -o -type l \) | wc -l)"
fi
if command -v restorecon >/dev/null 2>&1; then
    sudo restorecon -R "$PREFIX" >/dev/null 2>&1 || true
fi

for tool in "$SRC"/bin/gnoblin*; do
    name="$(basename "$tool")"
    sudo mkdir -p "$BIN_DIR"
    sudo ln -sfn "$PREFIX/bin/$name" "$BIN_DIR/$name"
    installed "$BIN_DIR/$name"
done

desktop_to_install="$(mktemp)"
trap 'rm -f -- "$desktop_to_install"' EXIT
for entry in "${public_entries[@]}"; do
    case "$entry" in
        share/wayland-sessions/*.desktop)
            # GDM must not wait for this session to register with GNOME Session.
            python3 - "$SRC/$entry" "$desktop_to_install" <<'PY'
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
            install_public_file "$desktop_to_install" "$SYSTEM_ROOT/$entry"
            ;;
        *)
            install_public_file "$SRC/$entry" "$SYSTEM_ROOT/$entry"
            ;;
    esac
done

# An earlier install linked these from the source directory. Such a link wins over the system copy and would run the
# old build, so remove the links that point into a Gnoblin prefix. Anything else belongs to the user and stays.
remove_old_link() {
    local link="$1" suffix="$2" target
    [ -L "$link" ] || return 0
    target="$(readlink "$link")"
    case "$target" in
        */"$suffix")
            rm "$link"
            echo "  Removed the old link $link -> $target"
            ;;
        *) echo "  Left $link alone: it points to $target, which is not a Gnoblin prefix." >&2 ;;
    esac
}
for entry in "${public_entries[@]}"; do
    case "$entry" in
        lib/systemd/user/*) remove_old_link "$USER_UNIT_DIR/${entry##*/}" "$entry" ;;
        share/man/man1/*) remove_old_link "$USER_MAN_DIR/${entry##*/}" "$entry" ;;
    esac
done
remove_old_link "$USER_UNIT_DIR/gnoblin-recovery.service" lib/systemd/user/gnoblin-recovery.service
remove_old_link "$USER_BIN_DIR/gnoblinctl" bin/gnoblinctl

if "$have_user_systemd"; then
    systemctl --user daemon-reload
else
    echo 'No systemd user manager detected; Gnoblin core will run without user units.'
fi

printf '\n%sGnoblin is installed%s. Choose it at the login screen.\n' "$green" "$reset"
if ! "$with_portal"; then
    echo 'Gnoblin portal backend not built; the default route will select another installed backend.'
    echo 'Set gnoblin.configure.portals in Lua to choose a specific backend.'
fi
