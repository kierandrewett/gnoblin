#!/usr/bin/env python3
"""List the Fedora releases that the package workflows build, from design/package-targets.json.

usage: scripts/fedora-releases.py [--github-output FILE] [--names released|prerelease]

Every target of the fedora family in the inventory whose status is existing-path (an automated path may build it) is
built, installed and packaged by .github/workflows/packages.yml. A target that is only planned or needs a probe is not.
To add a release, promote a candidate of scripts/release-candidates.json by adding its target to the inventory; no
workflow changes. A target whose lifecycle is upstream-pre-release is a pre-release: its jobs may fail without failing
the run, because the release is not out yet.

The script prints one JSON object:

  releases      the releases, oldest first: {"release": "43", "image": "fedora:43", "prerelease": false}
  source        the release whose prefix also makes the openSUSE rpm, deb, Arch and tar packages: the target with
                "shared_prefix": true. Those packages run on systems that are newer than Fedora, so the prefix must link
                the libraries that those systems have (Fedora 43 links libdisplay-info.so.2, the others .so.3), and it
                must not be a pre-release, so those packages never wait on one. A glibc package asks for the glibc of the
                host that built it, so the oldest release that fits gives the lowest floor
  source_image  the container image of that release

With --names the script prints only the release numbers of the releases that are out, or of the pre-releases, separated
by commas ("43, 44"), for the release notes.

With --github-output the script also appends releases=, source= and source_image= lines to FILE, which is how a workflow
reads them.
"""

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent.parent
INVENTORY = ROOT / "design/package-targets.json"
PRE_RELEASE = "upstream-pre-release"


def fedora_releases(inventory):
    releases = []
    for target in inventory["targets"]:
        if target["family"] != "fedora" or target["status"] != "existing-path":
            continue
        release = target["release"]
        if not release.isdigit():
            raise ValueError(f"{target['id']}: the Fedora release {release!r} is not a number")
        if target["id"] != f"fedora-{release}" or target["image"] != f"fedora:{release}":
            raise ValueError(f"{target['id']}: the id and the image must name the release {release}")
        releases.append(
            {"release": release, "image": target["image"], "prerelease": target["lifecycle"] == PRE_RELEASE}
        )
    releases.sort(key=lambda item: int(item["release"]))
    if len({item["release"] for item in releases}) != len(releases):
        raise ValueError("a Fedora release is listed twice")
    return releases


def describe(inventory):
    releases = fedora_releases(inventory)
    built = {item["release"] for item in releases}
    shared = [
        target["release"]
        for target in inventory["targets"]
        if target["family"] == "fedora" and target["release"] in built and target.get("shared_prefix")
    ]
    if len(shared) != 1:
        raise ValueError('exactly one fedora target must have "shared_prefix": true')
    source = next(item for item in releases if item["release"] == shared[0])
    if source["prerelease"]:
        raise ValueError(
            f"Fedora {source['release']} is a pre-release and cannot make the packages of the other distributions"
        )
    return {"releases": releases, "source": source["release"], "source_image": source["image"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--github-output", type=Path, help="append the values to this GITHUB_OUTPUT file")
    parser.add_argument("--names", choices=("released", "prerelease"), help="print only these release numbers")
    arguments = parser.parse_args()
    try:
        description = describe(json.loads(INVENTORY.read_text()))
    except (OSError, KeyError, ValueError) as exception:
        print(f"fedora releases: {exception}", file=sys.stderr)
        return 1
    if arguments.names:
        wanted = arguments.names == "prerelease"
        print(", ".join(item["release"] for item in description["releases"] if item["prerelease"] == wanted))
        return 0
    print(json.dumps(description, indent=2))
    if arguments.github_output:
        with arguments.github_output.open("a") as output:
            output.write(f"releases={json.dumps(description['releases'], separators=(',', ':'))}\n")
            output.write(f"source={description['source']}\nsource_image={description['source_image']}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
