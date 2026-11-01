#!/usr/bin/env python3
"""Give a CI checkout a nightly version, so the source bundle and the programs built from it carry it.

usage: apply-nightly-version.py SEMVER

SEMVER is a pre-release such as 0.1.13-nightly.20261031.142.gabc1234 (`python3 scripts/pkgrepo version` prints it).
The script writes it into src/gnoblin-version.json, which the source bundle, `gnoblin --version` and
scripts/build-packages.sh read. pkgsmith spells the version for each package format (0.1.13~nightly... for rpm and deb,
0.1.13nightly20261031.142 for pacman). Release builds never run it; it exists for CI checkouts that are thrown away.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_FILE = ROOT / "src/gnoblin-version.json"
NIGHTLY = re.compile(r"^\d+\.\d+\.\d+-nightly\.\d{8}\.\d+\.g[0-9a-f]{7}$")


def main() -> int:
    if len(sys.argv) != 2 or not NIGHTLY.match(sys.argv[1]):
        print(__doc__, file=sys.stderr)
        return 2
    version = json.loads(VERSION_FILE.read_text())
    VERSION_FILE.write_text(json.dumps({**version, "version": sys.argv[1]}, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
