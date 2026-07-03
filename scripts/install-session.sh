#!/usr/bin/env bash
# Install gnoblin's session data into a prefix:
#   - the `gnoblin` GNOME Shell session mode (strips the stock UI declaratively)
#   - the gnome-session definition (required components)
#   - the wayland-session .desktop entry (shown at the login manager)
#
# This is additive and atomic: everything lands under <prefix>/share and can be
# removed by deleting these files. No system files are touched here.
set -euo pipefail

PREFIX="${1:?usage: install-session.sh <prefix>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src/data/session"

install -Dm644 "$SRC/modes/gnoblin.json" \
  "$PREFIX/share/gnome-shell/modes/gnoblin.json"
install -Dm644 "$SRC/gnome-session/gnoblin.session" \
  "$PREFIX/share/gnome-session/sessions/gnoblin.session"
install -Dm644 "$SRC/gnoblin.desktop" \
  "$PREFIX/share/wayland-sessions/gnoblin.desktop"

# The gnoblinctl CLI (org.gnoblin.Shell control front-end).
install -Dm755 "$ROOT/src/tools/gnoblinctl" "$PREFIX/bin/gnoblinctl"

echo ">> installed gnoblin session data into $PREFIX/share:"
echo "     gnome-shell/modes/gnoblin.json         (UI-strip session mode)"
echo "     gnome-session/sessions/gnoblin.session (required components)"
echo "     wayland-sessions/gnoblin.desktop        (login entry)"
echo "     bin/gnoblinctl                          (control CLI)"
