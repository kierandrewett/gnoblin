#!/usr/bin/env bash
# Register the gnoblin session with your login manager / systemd --user
# instance. Separate from install-session.sh (and NOT run by `just dev` /
# `just dev-session`) because it touches state outside the prefix:
#
#   1. Links org.gnoblin.Shell.target + org.gnoblin.Shell@wayland.service
#      into systemd --user's search path (~/.config/systemd/user/), so
#      gnome-session's RequiredComponents=org.gnoblin.Shell can find them.
#      User-level, no root, fully reversible (see "Undo" below). These are
#      gnoblin-specific unit names -- this does NOT touch or shadow
#      org.gnome.Shell@wayland.service, so any system GNOME session is
#      unaffected.
#   2. Prints (does not run) the command to make "Gnoblin" appear at your
#      login manager's session picker. That step needs root, because login
#      managers read wayland-session .desktop files from a fixed system
#      directory (e.g. /usr/share/wayland-sessions/), not anything under
#      your prefix or home directory -- there's no way around that without
#      a real package install (see `just rpm`).
#
# Usage: register-session.sh <prefix>   (run install-session.sh <prefix> first)
set -euo pipefail

PREFIX="${1:?usage: register-session.sh <prefix>}"
PREFIX="$(cd "$PREFIX" && pwd)"
UNIT_DIR="$PREFIX/lib/systemd/user"
TARGET="$UNIT_DIR/org.gnoblin.Shell.target"
SERVICE="$UNIT_DIR/org.gnoblin.Shell@wayland.service"
DESKTOP="$PREFIX/share/wayland-sessions/gnoblin.desktop"

for f in "$TARGET" "$SERVICE" "$DESKTOP"; do
  [ -f "$f" ] || { echo "missing $f -- run ./scripts/install-session.sh $PREFIX first" >&2; exit 1; }
done

command -v systemctl >/dev/null 2>&1 || { echo "systemctl not found -- this needs a systemd user session" >&2; exit 1; }

echo ">> linking gnoblin's systemd --user units (does not touch org.gnome.Shell*):"
systemctl --user link "$TARGET" "$SERVICE"
systemctl --user daemon-reload
echo "     org.gnoblin.Shell.target"
echo "     org.gnoblin.Shell@wayland.service"

cat <<EOF

>> systemd --user units linked. Verify with:
     systemctl --user list-unit-files | grep gnoblin

>> Still needed to see "Gnoblin" at your login manager's session picker --
   run this yourself (needs root; not run automatically):

     sudo install -Dm644 "$DESKTOP" /usr/share/wayland-sessions/gnoblin.desktop

>> Undo, any time:
     rm ~/.config/systemd/user/org.gnoblin.Shell.target
     rm ~/.config/systemd/user/org.gnoblin.Shell@wayland.service
     systemctl --user daemon-reload
     sudo rm /usr/share/wayland-sessions/gnoblin.desktop
EOF
