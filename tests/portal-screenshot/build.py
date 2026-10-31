#!/usr/bin/env python3
"""Build the complete portal from pinned sources in a new disposable directory.

No install, submodule edits, service changes or network fetches. Requires the
pinned upstream objects and build dependencies already present locally.
"""

import argparse
import configparser
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[2]


def archive(repo, revision, destination):
    destination.mkdir(parents=True, exist_ok=False)
    data = subprocess.check_output(["git", "-C", str(repo), "archive", revision])
    with tarfile.open(fileobj=io.BytesIO(data)) as source:
        source.extractall(destination, filter="data")


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new build directory")
    parser.add_argument("--prefix", default="/usr/local/lib/gnoblin")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    upstream = ROOT / "subprojects/xdg-desktop-portal-gnome"
    revision = json.loads((ROOT / "src/gnome-versions.json").read_text())["components"]["xdg-desktop-portal-gnome"][
        "commit"
    ]
    archive(upstream, revision, source)
    run(
        "cmake",
        "-DACTION=overlay",
        "-DPROJECT=xdg-desktop-portal-gnome",
        "-DMODE=copy",
        "-DSOURCE_DIR=" + str(source),
        "-P",
        ROOT / "cmake/source-step.cmake",
    )
    for patch in sorted((ROOT / "patches/xdg-desktop-portal-gnome").rglob("*.patch")):
        run("git", "-C", source, "apply", patch)
    wrap = configparser.ConfigParser()
    wrap.read(source / "subprojects/libgxdp.wrap")
    archive(upstream / "subprojects/libgxdp", wrap["wrap-git"]["revision"], source / "subprojects/libgxdp")
    for patch in sorted((ROOT / "patches/portal-dependencies/libgxdp").glob("*.patch")):
        run("git", "-C", source / "subprojects/libgxdp", "apply", patch)
    run(
        "meson",
        "setup",
        output / "build",
        source,
        "--wrap-mode=nodownload",
        "--prefix=" + args.prefix,
        "-Dsystemd=disabled",
    )
    run("meson", "compile", "-C", output / "build", "-j", "4")
    run("meson", "test", "-C", output / "build", "--print-errorlogs")
    print("Built, not installed:", output / "build/src/xdg-desktop-portal-gnoblin")


if __name__ == "__main__":
    main()
