#!/usr/bin/env bash
# Stage the loose, Gnoblin-owned RPM Source files for one patched subproject.
set -euo pipefail

PROJECT="${1:?usage: stage-rpm-sources.sh <mutter|gnome-shell> <outdir>}"
OUTDIR="${2:?usage: stage-rpm-sources.sh <mutter|gnome-shell> <outdir>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

install -d -- "$OUTDIR"

case "$PROJECT" in
    mutter)
        install -m 0644 -- \
            "$ROOT/packaging/rpm/org.gnome.mutter.fedora.gschema.override" \
            "$OUTDIR/org.gnome.mutter.fedora.gschema.override"
        ;;
    gnome-shell)
        install -m 0644 -- "$ROOT/src/data/session/modes/gnoblin.json" "$OUTDIR/gnoblin.json"
        install -m 0644 -- "$ROOT/src/data/session/gnome-session/gnoblin.session" "$OUTDIR/gnoblin.session"
        install -m 0644 -- "$ROOT/src/data/session/gnoblin.desktop" "$OUTDIR/gnoblin.desktop"
        install -m 0644 -- \
            "$ROOT/src/data/session/schemas/00_org.gnoblin.mutter.gschema.override" \
            "$OUTDIR/00_org.gnoblin.mutter.gschema.override"
        install -m 0644 -- \
            "$ROOT/src/data/session/systemd-user/org.gnoblin.Shell.target" \
            "$OUTDIR/org.gnoblin.Shell.target"
        install -m 0644 -- \
            "$ROOT/src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in" \
            "$OUTDIR/org.gnoblin.Shell@wayland.service.in"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-env.sh" "$OUTDIR/gnoblin-env.sh"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-session" "$OUTDIR/gnoblin-session"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-shell-service" "$OUTDIR/gnoblin-shell-service"
        install -m 0644 -- "$ROOT/src/tools/gnoblinctl" "$OUTDIR/gnoblinctl"
        ;;
    *)
        echo "unknown RPM source project: $PROJECT" >&2
        exit 1
        ;;
esac
