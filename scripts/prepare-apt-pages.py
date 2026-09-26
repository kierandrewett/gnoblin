#!/usr/bin/env python3
"""Prepare the APT archive for Cloudflare Pages and redirect DEBs to releases."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


# Keep these entries aligned with build-apt-repository.py's supported suites.
SUITES = (
    ("debian", "13", "debian13", "gnoblin-debian13-amd64.deb"),
    ("ubuntu", "24.04", "ubuntu24.04", "gnoblin-ubuntu24.04-amd64.deb"),
    ("ubuntu", "26.04", "ubuntu26.04", "gnoblin-ubuntu26.04-amd64.deb"),
)
SEMVER_RE = re.compile(r"\+gnoblin(\d+\.\d+\.\d+)-\d+~([a-zA-Z0-9.]+)$")


def stanzas(path: Path) -> list[dict[str, str]]:
    content = path.read_text(encoding="utf-8").strip()
    result: list[dict[str, str]] = []
    for paragraph in content.split("\n\n"):
        fields: dict[str, str] = {}
        for line in paragraph.splitlines():
            if line[:1].isspace():
                continue
            key, separator, value = line.partition(":")
            if separator:
                fields[key] = value.strip()
        result.append(fields)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="APT archive root copied into the static site")
    parser.add_argument("--output", type=Path, required=True, help="Cloudflare Pages _redirects output file")
    args = parser.parse_args()

    redirects: dict[str, str] = {}
    for archive, suite, suffix, asset in SUITES:
        index = args.archive / archive / "dists" / suite / "main/binary-amd64/Packages"
        if not index.is_file():
            raise SystemExit(f"missing APT index: {index}")
        for fields in stanzas(index):
            if fields.get("Package") != "gnoblin":
                continue
            version = fields.get("Version", "")
            match = SEMVER_RE.search(version)
            filename = fields.get("Filename", "")
            if not match or match.group(2) != suffix:
                raise SystemExit(f"cannot map APT package version to a release: {version}")
            if not filename.startswith("pool/") or ".." in Path(filename).parts:
                raise SystemExit(f"unsafe APT package path: {filename}")
            route = f"/apt/{archive}/{filename}"
            destination = (
                f"https://github.com/kierandrewett/gnoblin/releases/download/gnoblin-v{match.group(1)}/{asset}"
            )
            if route in redirects and redirects[route] != destination:
                raise SystemExit(f"conflicting APT download redirects: {route}")
            redirects[route] = destination

    if not redirects:
        raise SystemExit("APT indexes contain no Gnoblin packages")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        "".join(f"{route} {destination} 302\n" for route, destination in sorted(redirects.items())),
        encoding="utf-8",
    )

    removed = 0
    for package in args.archive.rglob("*.deb"):
        package.unlink()
        removed += 1
    print(f"wrote {len(redirects)} GitHub release redirects and removed {removed} oversized DEBs")


if __name__ == "__main__":
    main()
