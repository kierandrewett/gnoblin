#!/usr/bin/env python3
"""Record GNOME 51 host-library capability floors from an RPM target image."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from typing import Any, Dict, List, Optional, Union


REQUIREMENTS = {
    "glib": {
        "capability": "pkgconfig(glib-2.0)",
        "minimum": "2.86.0",
        "declaredScope": "host-runtime-contract",
        "floorSource": "subprojects/gnome-shell/meson.build:24",
    },
    "gjs": {
        "capability": "pkgconfig(gjs-1.0)",
        "minimum": "1.87.1",
        "declaredScope": "host-runtime-contract",
        "floorSource": "subprojects/gnome-shell/meson.build:25",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/gnome-shell.spec:45,66",
                "kind": "BuildRequires and Requires",
                "minimum": "1.87.1",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/gnome-shell.spec:60,116",
                "kind": "BuildRequires and Requires",
                "minimum": "1.87.1",
                "note": "matches-source-floor",
            },
        ],
    },
    "wayland": {
        "capability": "pkgconfig(wayland-client)",
        "minimum": "1.26.0",
        "declaredScope": "host-runtime-contract",
        "floorSource": "subprojects/mutter/meson.build:49,215",
    },
    "wayland-protocols": {
        "capability": "pkgconfig(wayland-protocols)",
        "minimum": "1.48",
        "declaredScope": "development-package-contract",
        "declaredPackage": "gnoblin-mutter-devel",
        "floorSource": "subprojects/mutter/meson.build:50,217-218",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/mutter.spec:20,74",
                "kind": "BuildRequires",
                "minimum": "1.48",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/mutter.spec:27,96",
                "kind": "BuildRequires",
                "minimum": "1.48",
                "note": "matches-source-floor",
            },
        ],
    },
    "libinput": {
        "capability": "pkgconfig(libinput)",
        "minimum": "1.30.0",
        "declaredScope": "host-runtime-contract",
        "floorSource": "patches/mutter/74-fedora43-compat/0001-input-allow-libinput-1.30.patch:19-20",
    },
    "pipewire": {
        "capability": "pkgconfig(libpipewire-0.3)",
        "minimum": "1.4.0",
        "declaredScope": "host-runtime-contract",
        "floorSource": "patches/mutter/74-fedora43-compat/0001-input-allow-libinput-1.30.patch:25-26",
    },
    "gtk4": {
        "capability": "pkgconfig(gtk4)",
        "minimum": "4.14.0",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/mutter/meson.build:23",
    },
    "girepository": {
        "capability": "pkgconfig(girepository-2.0)",
        "minimum": "2.86.0",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/gnome-shell/meson.build:27,78",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/gnome-shell.spec:43",
                "kind": "BuildRequires",
                "minimum": "2.86.0",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/gnome-shell.spec:61,83",
                "kind": "BuildRequires",
                "minimum": "2.86.0",
                "note": "matches-source-floor",
            },
        ],
    },
    "gcr4": {
        "capability": "pkgconfig(gcr-4)",
        "minimum": "3.90.0",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/gnome-shell/meson.build:26,76",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/gnome-shell.spec:41",
                "kind": "BuildRequires",
                "minimum": "3.90.0",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/gnome-shell.spec:62,82",
                "kind": "BuildRequires",
                "minimum": "3.90.0",
                "note": "matches-source-floor",
            },
        ],
    },
    "glycin": {
        "capability": "pkgconfig(glycin-2)",
        "minimum": "2.0.beta.2",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/mutter/meson.build:24,121",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/mutter.spec:47",
                "kind": "BuildRequires",
                "minimum": "2.0.beta.2",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/mutter.spec:15,89",
                "kind": "BuildRequires",
                "minimum": "2.0.beta.2",
                "note": "matches-source-floor",
            },
        ],
    },
    "hyprcursor": {
        "capability": "pkgconfig(hyprcursor)",
        "minimum": "0.1.11",
        "declaredScope": "build-closure",
        "floorSource": "patches/mutter/43-hyprcursor/0001-cursor-themes-and-launch-feedback.patch:38",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/mutter.spec:54",
                "kind": "BuildRequires",
                "minimum": "0.1.11",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/mutter.spec:16,63",
                "kind": "BuildRequires",
                "minimum": "0.1.11",
                "note": "matches-source-floor",
            },
        ],
    },
    "libei": {
        "capability": "pkgconfig(libei-1.0)",
        "minimum": "1.3.901",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/mutter/meson.build:41,146",
    },
    "libeis": {
        "capability": "pkgconfig(libeis-1.0)",
        "minimum": "1.3.901",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/mutter/meson.build:41,145",
    },
    "libdisplay-info": {
        "capability": "pkgconfig(libdisplay-info)",
        "minimum": "0.2",
        "declaredScope": "build-closure",
        "floorSource": "subprojects/mutter/meson.build:42,148",
        "rpmSpecDeclarations": [
            {
                "location": "packaging/opensuse/mutter.spec:58",
                "kind": "BuildRequires",
                "minimum": "0.2",
                "note": "matches-source-floor",
            },
            {
                "location": "packaging/rpm/mutter.spec:19,73",
                "kind": "BuildRequires",
                "minimum": "0.2",
                "note": "matches-source-floor",
            },
        ],
    },
}


def command(*args: str) -> str:
    return subprocess.check_output(args, universal_newlines=True, env={**os.environ, "LC_ALL": "C"})


def os_release() -> Dict[str, str]:
    result = {}  # type: Dict[str, str]
    for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if separator:
            result[key] = value.strip().strip('"')
    return result


def version_tokens(value: str) -> List[Union[int, str]]:
    return [int(part) if part.isdigit() else part.lower() for part in re.findall(r"\d+|[A-Za-z]+", value)]


def version_at_least(observed: str, minimum: str) -> bool:
    """Compare the simple upstream version floors used by the package specs."""
    left, right = version_tokens(observed), version_tokens(minimum)
    for current, required in zip(left, right):
        if current == required:
            continue
        if isinstance(current, int) and isinstance(required, int):
            return current > required
        if isinstance(current, str) and isinstance(required, str):
            return current > required
        return isinstance(current, int)
    return len(left) >= len(right)


def dnf_candidates(capability: str) -> List[Dict[str, str]]:
    output = command(
        "dnf",
        "--quiet",
        "repoquery",
        "--available",
        "--latest-limit",
        "1",
        "--qf",
        "%{name}|%{epoch}|%{version}|%{release}",
        "--whatprovides",
        capability,
    )
    result = []
    for line in output.splitlines():
        name, separator, rest = line.partition("|")
        if not separator:
            continue
        fields = rest.split("|")
        if len(fields) != 3:
            continue
        result.append({"package": name, "epoch": fields[0], "version": fields[1], "release": fields[2]})
    return result


def zypper_candidates(capability: str) -> List[Dict[str, str]]:
    try:
        output = command("zypper", "--non-interactive", "search", "--details", "--provides", capability)
    except subprocess.CalledProcessError as error:
        # zypper returns 104 when a well-formed capability has no provider.
        if error.returncode == 104:
            return []
        raise
    result = []
    for line in output.splitlines():
        fields = [field.strip() for field in line.split("|")]
        if len(fields) != 6 or fields[0] or fields[2] != "package" or fields[1] == "Name":
            continue
        result.append({"package": fields[1], "epoch": "0", "version": fields[3], "release": ""})
    return result


def select_latest(candidates: List[Dict[str, str]]) -> Optional[Dict[str, str]]:
    if not candidates:
        return None
    latest = candidates[0]
    for candidate in candidates[1:]:
        if version_at_least(candidate["version"], latest["version"]):
            latest = candidate
    return latest


def probe() -> Dict[str, Any]:
    distro = os_release()
    identifier = distro.get("ID")
    if identifier in {"rocky", "rhel", "almalinux", "centos"}:
        resolver = dnf_candidates
        family = "el"
    elif identifier in {"opensuse-leap", "opensuse-tumbleweed"}:
        resolver = zypper_candidates
        family = "opensuse"
    else:
        raise RuntimeError(f"unsupported RPM probe image: {identifier!r}")

    observed = {}  # type: Dict[str, Optional[Dict[str, str]]]
    blockers = []  # type: List[Dict[str, str]]
    for component, requirement in REQUIREMENTS.items():
        capability = requirement["capability"]
        minimum = requirement["minimum"]
        candidate = select_latest(resolver(capability))
        observed[component] = candidate
        if candidate is None:
            blockers.append({"component": component, "capability": capability, "reason": "not available"})
        elif not version_at_least(candidate["version"], minimum):
            blockers.append(
                {
                    "component": component,
                    "capability": capability,
                    "reason": f"{candidate['version']} is below {minimum}",
                }
            )
    return {
        "distribution": {"id": identifier, "version": distro.get("VERSION_ID")},
        "family": family,
        "requirements": REQUIREMENTS,
        "observed": observed,
        "blockers": blockers,
        "result": "blocked" if blockers else "ready-for-package-build",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    expected = parser.add_mutually_exclusive_group(required=True)
    expected.add_argument("--expect-blocked", action="store_true")
    expected.add_argument("--expect-ready", action="store_true")
    args = parser.parse_args()

    try:
        report = probe()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text, encoding="utf-8")
    print(text, end="")
    blocked = bool(report["blockers"])
    if args.expect_blocked != blocked:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
