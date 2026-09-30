#!/usr/bin/env bash
# Stage the loose, Gnoblin-owned RPM Source files for one patched subproject.
set -euo pipefail

PROJECT="${1:?usage: stage-rpm-sources.sh <mutter|xdg-desktop-portal-gnome> <outdir>}"
OUTDIR="${2:?usage: stage-rpm-sources.sh <mutter|xdg-desktop-portal-gnome> <outdir>}"
install -d -- "$OUTDIR"

case "$PROJECT" in
    mutter | xdg-desktop-portal-gnome)
        ;;
    *)
        echo "unknown RPM source project: $PROJECT" >&2
        exit 1
        ;;
esac
