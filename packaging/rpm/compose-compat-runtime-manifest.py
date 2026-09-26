#!/usr/bin/env python3
"""Compose the pinned private GNOME runtime used by legacy RPM targets."""

import argparse
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFESTS = (
    ROOT / "packaging/rpm/compat-bootstrap.json",
    ROOT / "packaging/deb/compat-bootstrap.json",
    ROOT / "packaging/deb/build-dependencies.json",
    ROOT / "build-dependencies.json",
)


def compose(manifests):
    """Return one dependency-ordered closure with no competing GLib recipe."""
    combined = []
    names = set()
    for manifest in manifests:
        recipes = json.loads(Path(manifest).read_text(encoding="utf-8"))
        if not isinstance(recipes, list):
            raise RuntimeError("private dependency manifest must contain a JSON array: {}".format(manifest))
        for recipe in recipes:
            name = recipe.get("name")
            if not isinstance(name, str) or not name:
                raise RuntimeError("private dependency recipe is missing its name: {}".format(manifest))
            # `glib-final` from the bootstrap manifest is the only GLib build
            # with private introspection enabled.  Adding the generic base
            # `glib` recipe afterwards would overwrite that interface.
            if name == "glib" and "glib-final" in names:
                continue
            if name in names:
                continue
            names.add(name)
            combined.append(recipe)

    required = {
        "patchelf",
        "glib-final",
        "gobject-introspection-bootstrap",
        "wayland",
        "wayland-protocols",
        "gtk4",
        "gcr4",
        "gjs",
        "glycin",
        "libei",
        "libdisplay-info",
        "hyprcursor",
        "mozjs",
        "gnome-desktop",
    }
    missing = required - names
    if missing:
        raise RuntimeError("incomplete private RPM runtime: {}".format(", ".join(sorted(missing))))
    return combined


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--manifest",
        type=Path,
        action="append",
        default=[],
        help="Input manifest, in dependency order. Defaults to the RPM compatibility closure.",
    )
    args = parser.parse_args()
    manifests = args.manifest or list(DEFAULT_MANIFESTS)
    try:
        result = compose(manifests)
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
