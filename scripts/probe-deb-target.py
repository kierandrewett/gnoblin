#!/usr/bin/env python3
"""Measure whether an older Debian-family image can build Gnoblin 51.

This is a diagnostic gate, not a package installer.  It records the current
stock repository state before a target is added to the Debian package build
matrix.  The current private runtime deliberately does not bundle GTK, GCR,
or GIRepository, so those interfaces must either exist at compatible versions
on the target or gain a separately reviewed private-runtime implementation.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys


TARGETS = {
    ("debian", "12"): "debian12",
    ("ubuntu", "22.04"): "ubuntu22.04",
}

# Mutter 51 uses GTK 4.14 APIs.  The remaining interfaces are absent from
# these target repositories and are not supplied by build-dependencies.json
# or packaging/deb/build-dependencies.json.
REQUIREMENTS = {
    "libgtk-4-dev": "4.14.0",
    "libgirepository-2.0-dev": None,
    "libgcr-4-dev": None,
    "libei-dev": None,
    "libeis-dev": None,
    "libdisplay-info-dev": None,
    "libglycin-2-dev": None,
    "libhyprcursor-dev": None,
}


def os_release() -> dict[str, str]:
    values: dict[str, str] = {}
    for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if separator:
            values[key] = value.strip().strip('"')
    return values


def candidate(package: str) -> str | None:
    result = subprocess.run(
        ["apt-cache", "show", package],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        return None
    for line in result.stdout.splitlines():
        if line.startswith("Version: "):
            return line.removeprefix("Version: ")
    return None


def version_at_least(version: str, minimum: str) -> bool:
    return subprocess.run(["dpkg", "--compare-versions", version, "ge", minimum], check=False).returncode == 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="write the JSON report to this path")
    parser.add_argument(
        "--expect-blocked",
        action="store_true",
        help="fail if this target unexpectedly satisfies the current host dependency boundary",
    )
    args = parser.parse_args()

    distro = os_release()
    identity = (distro.get("ID"), distro.get("VERSION_ID"))
    target = TARGETS.get(identity)
    if target is None:
        parser.error("only Debian 12 and Ubuntu 22.04 are probe targets")

    subprocess.run(["apt-get", "update"], check=True)
    observed: dict[str, str | None] = {package: candidate(package) for package in REQUIREMENTS}
    blockers: list[dict[str, str]] = []
    for package, minimum in REQUIREMENTS.items():
        version = observed[package]
        if version is None:
            blockers.append({"package": package, "reason": "not available from the stock target repositories"})
        elif minimum is not None and not version_at_least(version, minimum):
            blockers.append({"package": package, "reason": f"{version} is below Gnoblin 51's required {minimum}"})

    report = {
        "target": target,
        "distribution": {"id": identity[0], "version": identity[1]},
        "private_runtime_boundary": [
            "GTK, GIRepository, and GCR stay outside the current private DEB runtime.",
            "libei, libdisplay-info, Glycin, and Hyprcursor can be built privately only after their build chain resolves.",
            "GDM/PAM, systemd/logind, gnome-session, GNOME Settings Daemon, D-Bus, PipeWire, WirePlumber, udev, and Mesa remain host-owned.",
        ],
        "requirements": observed,
        "blockers": blockers,
        "result": "blocked" if blockers else "ready-for-private-runtime-design",
    }
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    print(text, end="")
    if args.expect_blocked and not blockers:
        print("target unexpectedly has no current-host blockers; review the private runtime boundary", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
