#!/usr/bin/env python3
"""Give a CI checkout a nightly version, so every recipe and the source bundle carry it.

usage: apply-nightly-version.py SEMVER

SEMVER is a pre-release such as 0.1.13-nightly.20261031.142.gabc1234 (`python3 scripts/pkgrepo version` prints it).
The script writes it into src/gnoblin-version.json and into the Gnoblin entries of
packaging/native-packages.json, then regenerates the Fedora spec and the Arch recipes, which spell it for their package
manager (0.1.13~nightly... for RPM, 0.1.13nightly20261031.142 for pacman). Release builds never run it; it exists for
CI checkouts that are thrown away.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_FILE = ROOT / "src/gnoblin-version.json"
MANIFEST = ROOT / "packaging/native-packages.json"
NIGHTLY = re.compile(r"^\d+\.\d+\.\d+-nightly\.\d{8}\.\d+\.g[0-9a-f]{7}$")


def main() -> int:
    if len(sys.argv) != 2 or not NIGHTLY.match(sys.argv[1]):
        print(__doc__, file=sys.stderr)
        return 2
    semver = sys.argv[1]
    version = json.loads(VERSION_FILE.read_text())
    VERSION_FILE.write_text(json.dumps({**version, "version": semver}, indent=2) + "\n")
    manifest = json.loads(MANIFEST.read_text())
    manifest["release"]["gnoblinVersion"] = semver
    for package in manifest["packages"].values():
        # Packages with a "source" take the version of that GNOME component, which a nightly does not change.
        if "source" not in package:
            package["version"] = semver
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    subprocess.run([sys.executable, str(ROOT / "scripts/sync-package-manifest.py"), "write"], check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
