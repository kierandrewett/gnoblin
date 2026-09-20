#!/usr/bin/env python3
"""Read Gnoblin's SemVer release identity and its GNOME compatibility train."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "gnoblin-version.json"
GNOME_MANIFEST = ROOT / "gnome-versions.json"
SEMVER = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def release() -> dict[str, str | int]:
    data = json.loads(MANIFEST.read_text())
    version = data.get("version")
    if data.get("format") != 1 or not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise RuntimeError(f"{MANIFEST.name}: expected format 1 and a SemVer version")
    gnome = json.loads(GNOME_MANIFEST.read_text())["components"]["gnome-shell"]["version"]
    return {"format": 1, "version": version, "gnomeVersion": gnome, "tag": f"gnoblin-v{version}"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("json", "get"))
    parser.add_argument("field", nargs="?", choices=("version", "gnomeVersion", "tag"))
    args = parser.parse_args()
    value = release()
    if args.command == "json":
        if args.field:
            parser.error("json does not take a field")
        print(json.dumps(value, sort_keys=True))
    else:
        if not args.field:
            parser.error("get requires a field")
        print(value[args.field])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
