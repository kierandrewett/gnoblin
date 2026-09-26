#!/usr/bin/env bash
# Build the complete private GNOME runtime and DEB package for older
# Debian-family targets whose host libraries are below the GNOME 51 interface
# floor.
set -euo pipefail

root="$(cd -- "$(dirname -- "$(realpath -- "$0")")/.." && pwd)"
cd -- "$root"

if [ "$(id -u)" -eq 0 ] || [ ! -w /usr/lib/gnoblin ]; then
    echo 'Run as the container build user after provision-deb-compat-container.sh.' >&2
    exit 2
fi

source /etc/os-release
case "$ID:$VERSION_ID" in
    debian:11 | debian:12 | ubuntu:22.04) ;;
    *)
        echo 'The compatibility runtime is limited to Debian 11/12 and Ubuntu 22.04.' >&2
        exit 2
        ;;
esac

if ! command -v rustc >/dev/null || ! command -v cargo >/dev/null; then
    echo 'The full compatibility graph needs a pinned Rust 1.85+ build toolchain for Glycin.' >&2
    exit 3
fi

rust_version="$(rustc --version | awk '{print $2}')"
python3 - "$rust_version" <<'PY'
import sys

parts = tuple(int(part) for part in sys.argv[1].split(".")[:2])
if parts < (1, 85):
    raise SystemExit(
        "The target Rust compiler is below Glycin 2.0's declared Rust 1.85 floor; "
        "do not substitute a host GNOME library."
    )
PY

mkdir -p build
python3 - <<'PY'
"""Compose the private GNOME compatibility graph without changing production DEB inputs."""
import json
from pathlib import Path

root = Path.cwd()
compat = json.loads((root / "packaging/deb/compat-bootstrap.json").read_text())
base = json.loads((root / "build-dependencies.json").read_text())
debian = json.loads((root / "packaging/deb/build-dependencies.json").read_text())

names = set()
for recipe in compat:
    if recipe["name"] in names:
        raise SystemExit(f"duplicate compatibility recipe: {recipe['name']}")
    names.add(recipe["name"])

# The compatibility graph supplies newer GLib, Wayland and Wayland Protocols.
# Retaining either copy would make the resulting private closure ambiguous.
combined = [*compat]
for recipe in [*debian, *base]:
    if recipe["name"] in names:
        continue
    names.add(recipe["name"])
    combined.append(recipe)

required = {
    "glib-final", "wayland", "wayland-protocols", "gtk4", "gcr4", "pango", "libheif", "glycin",
    "libei", "libdisplay-info", "hyprcursor", "mozjs", "gjs", "gnome-desktop",
}
missing = required - names
if missing:
    raise SystemExit(f"incomplete old-target runtime graph: {', '.join(sorted(missing))}")

(root / "build/compat-runtime-dependencies.json").write_text(
    json.dumps(combined, indent=2) + "\n"
)
PY

just init
python3 scripts/build-private-deps.py \
    --prefix /usr/lib/gnoblin/deps \
    --manifest build/compat-runtime-dependencies.json \
    --jobs "${GNOBLIN_BUILD_JOBS:-2}"
python3 scripts/build-private-deps.py --prefix /usr/lib/gnoblin/deps --run \
    env GNOBLIN_PREFIX=/usr/lib/gnoblin GNOBLIN_LIBDIR=lib64 GNOBLIN_DEVKIT=disabled \
    just build-local
python3 scripts/build-private-deps.py --prefix /usr/lib/gnoblin/deps \
    --fix-runtime --runtime-prefix /usr/lib/gnoblin
python3 scripts/package-deb.py "$@"
