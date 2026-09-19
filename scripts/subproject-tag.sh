#!/usr/bin/env bash
# Print the pinned upstream tag for a subproject. Single source of truth for
# the reset/patch/tag-fetch steps; keep in sync with the submodule pins.
set -euo pipefail

PROJ="${1:?usage: subproject-tag.sh <project>}"

case "$PROJ" in
    mutter) echo "49.5" ;;
    gnome-shell) echo "49.6" ;;
    gnome-control-center) echo "49.6" ;;
    xdg-desktop-portal-gnome) echo "49.0" ;;
    *)
        echo "unknown subproject: $PROJ" >&2
        exit 1
        ;;
esac
