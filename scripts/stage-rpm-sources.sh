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
        install -m 0644 -- \
            "$ROOT/src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf" \
            "$OUTDIR/gnome-session@gnoblin.target.d.conf"
        python3 - "$ROOT/build-dependencies.json" "$ROOT/packaging/rpm/gnome-shell.spec" "$OUTDIR" <<'PY'
import hashlib
import json
import pathlib
import re
import sys
import urllib.request

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
recipe = next(item for item in manifest if item["name"] == "gjs")
spec = pathlib.Path(sys.argv[2]).read_text()
match = re.search(r"^%global gjs_version ([0-9.]+)$", spec, re.MULTILINE)
if not match:
    raise SystemExit("RPM spec must pin its private GJS source version")
version = match.group(1)
if recipe["version"] != version:
    raise SystemExit("RPM private GJS version must match build-dependencies.json")
destination = pathlib.Path(sys.argv[3]) / f"gjs-{version}.tar.xz"
if not destination.exists():
    temporary = destination.with_suffix(destination.suffix + ".part")
    with urllib.request.urlopen(recipe["url"], timeout=60) as source, temporary.open("wb") as target:
        while chunk := source.read(1024 * 1024):
            target.write(chunk)
    temporary.replace(destination)
digest = hashlib.sha256(destination.read_bytes()).hexdigest()
if digest != recipe["sha256"]:
    destination.unlink(missing_ok=True)
    raise SystemExit("Pinned GJS source checksum mismatch")
PY
        install -m 0644 -- "$ROOT/src/tools/gnoblin-env.sh" "$OUTDIR/gnoblin-env.sh"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-session" "$OUTDIR/gnoblin-session"
        install -m 0644 -- "$ROOT/COPYING" "$OUTDIR/gnoblin-COPYING"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-seed-config" "$OUTDIR/gnoblin-seed-config"
        install -m 0644 -- "$ROOT/src/data/init.lua.example" "$OUTDIR/init.lua.example"
        install -m 0644 -- "$ROOT/src/tools/gnoblin-shell-service" "$OUTDIR/gnoblin-shell-service"
        install -m 0644 -- "$ROOT/src/tools/gnoblinctl" "$OUTDIR/gnoblinctl"
        theme_build="$(mktemp -d)"
        trap 'rm -rf -- "$theme_build"' EXIT
        python3 "$ROOT/scripts/build-adwaita-hyprcursor.py" \
            --output "$theme_build/Adwaita-Hyprcursor" \
            --fallback "${ADWAITA_CURSOR_FALLBACK:-/usr/share/icons/Adwaita}"
        tar -C "$theme_build" -cJf "$OUTDIR/Adwaita-Hyprcursor.tar.xz" Adwaita-Hyprcursor
        ;;
    *)
        echo "unknown RPM source project: $PROJECT" >&2
        exit 1
        ;;
esac
