#!/usr/bin/env python3
"""Materialize the Nix-owned native-package interface for distro adapters."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "packaging/generated/manifest.json"


def evaluate() -> str:
    result = subprocess.run(
        ["nix", "eval", "--json", ".#lib.nativePackages"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    rendered = json.dumps(json.loads(result.stdout), indent=2, sort_keys=True)

    # Match Prettier's compact representation for short arrays of strings so
    # the generated snapshot also passes the repository-wide formatter check.
    def compact_strings(match: re.Match) -> str:
        values = [line.strip().removesuffix(",") for line in match.group(1).splitlines()]
        compact = "[" + ", ".join(values) + "]"
        return compact if len(compact) <= 120 else match.group(0)

    rendered = re.sub(r"\[\n((?:\s+\"[^\n]+\",?\n)+)\s+\]", compact_strings, rendered)
    return rendered + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("write", "check"))
    args = parser.parse_args()
    rendered = evaluate()

    if args.command == "write":
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(rendered)
        print(f"wrote {OUTPUT.relative_to(ROOT)}")
        return 0

    if not OUTPUT.exists() or OUTPUT.read_text() != rendered:
        print(
            "packaging/generated/manifest.json is stale; run scripts/sync-package-manifest.py write",
            file=sys.stderr,
        )
        return 1
    print("native package manifest matches Nix")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
