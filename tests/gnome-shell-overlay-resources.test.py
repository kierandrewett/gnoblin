#!/usr/bin/env python3
"""Ensure every Gnoblin JavaScript overlay is in GNOME Shell's GResource."""

from __future__ import annotations

import sys
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RESOURCE_MANIFEST = ROOT / "subprojects/gnome-shell/js/js-resources.gresource.xml"


def main() -> int:
    if not RESOURCE_MANIFEST.is_file():
        print(f"missing patched resource manifest: {RESOURCE_MANIFEST}", file=sys.stderr)
        return 1

    resource_entries = [(element.text or "").strip() for element in ET.parse(RESOURCE_MANIFEST).iter("file")]
    duplicates = sorted(name for name, count in Counter(resource_entries).items() if count > 1)
    if duplicates:
        print("duplicate JavaScript resources in js-resources.gresource.xml:", file=sys.stderr)
        for resource in duplicates:
            print(f"  {resource}", file=sys.stderr)
        return 1

    resources = set(resource_entries)
    overlays: list[tuple[Path, str]] = []
    for manifest in sorted((ROOT / "src").rglob("manifest")):
        for line in manifest.read_text().splitlines():
            fields = line.split()
            if len(fields) < 3 or fields[0] != "gnome-shell":
                continue
            destination = fields[2]
            if destination.startswith("js/") and destination.endswith(".js"):
                overlays.append((manifest, destination.removeprefix("js/")))

    missing = sorted({resource for _, resource in overlays if resource not in resources})
    if missing:
        print("Gnoblin JavaScript overlays missing from js-resources.gresource.xml:", file=sys.stderr)
        for resource in missing:
            print(f"  {resource}", file=sys.stderr)
        return 1

    print(f"PASS: all {len(overlays)} Gnoblin JavaScript overlay files are bundled")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
