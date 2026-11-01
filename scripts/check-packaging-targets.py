#!/usr/bin/env python3
"""Validate the canonical Gnoblin package target inventory.

Run this after changing scripts/package-targets.json. The inventory records desired
targets and the evidence that exists today; an image or a CI path never by
itself marks a target as supported. It also prints warnings, which do not fail the check, when the inventory and
scripts/release-candidates.json disagree: a candidate without an entry, or an entry past its end of life.
"""

import datetime
import importlib.util
import json
import os
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent.parent
INVENTORY = ROOT / "scripts/package-targets.json"
CANDIDATES = ROOT / "scripts/release-candidates.json"
FAMILIES = {"fedora", "el", "debian", "ubuntu", "arch", "opensuse-leap", "opensuse-tumbleweed", "nixos"}
STATUSES = {"existing-path", "probe-needed", "planned"}
SUPPORT = {"unsupported", "candidate", "supported"}
GATES = {"dependency-probe", "package-build", "install", "coinstall", "graphical-session", "removal"}


def error(message):
    print(f"packaging target inventory: {message}", file=sys.stderr)
    return 1


def warn(message):
    """A warning does not fail the check: it is for a person to read, and for the weekly candidate update."""
    prefix = "::warning::" if os.environ.get("GITHUB_ACTIONS") else "warning: "
    print(f"{prefix}packaging target inventory: {message}", file=sys.stderr)


def candidate_warnings(inventory, candidates, today):
    """What a person should look at: candidates the inventory does not know, and inventory entries that fell behind.

    scripts/update-release-matrix.py writes the candidates and never edits the inventory. A person promotes a candidate
    by adding an inventory entry, so the two lists meet here.
    """
    warnings = []
    listed = {item["release"]: item for item in candidates.get("candidates", []) if item["family"] == "fedora"}
    entries = {t["release"]: t for t in inventory["targets"] if t["family"] == "fedora"}
    for release, item in sorted(listed.items(), key=lambda pair: int(pair[0])):
        if release not in entries and item["eligible"]:
            warnings.append(
                f"Fedora {release} is a release candidate that meets the build floors and has no inventory entry"
            )
        if release in entries and entries[release]["status"] == "existing-path" and not item["eligible"]:
            warnings.append(f"Fedora {release} is built but does not meet the build floors: {item['evidence']}")
        pre = entries.get(release, {}).get("lifecycle") == "upstream-pre-release"
        if release in entries and pre != item["prerelease"]:
            warnings.append(
                f"Fedora {release} lifecycle is {entries[release]['lifecycle']}, but the candidate list says prerelease={item['prerelease']}"
            )
    for release, target in sorted(entries.items(), key=lambda pair: int(pair[0])):
        item = listed.get(release)
        eol = datetime.date.fromisoformat(item["eol"]) if item and item["eol"] else None
        if item is None:
            warnings.append(
                f"Fedora {release} has an inventory entry but is no longer a release candidate: past its end of life or unobserved"
            )
        elif eol and eol <= today:
            warnings.append(f"Fedora {release} is past its end of life ({eol})")
    for item in candidates.get("needs_review", []):
        warnings.append(
            f"Fedora {item['release']} needs review: the sources disagree ({'; '.join(item['observations'].values())})"
        )
    return warnings


def main():
    try:
        inventory = json.loads(INVENTORY.read_text())
    except (OSError, json.JSONDecodeError) as exception:
        return error(f"cannot read {INVENTORY}: {exception}")

    if inventory.get("schema_version") != 1:
        return error("schema_version must be 1")
    targets = inventory.get("targets")
    if not isinstance(targets, list) or not targets:
        return error("targets must be a non-empty list")

    target_ids = set()
    found_families = set()
    failed = False
    for target in targets:
        if not isinstance(target, dict):
            failed |= bool(error("each target must be an object"))
            continue
        identifier = target.get("id")
        if not isinstance(identifier, str) or not identifier:
            failed |= bool(error("each target needs a non-empty id"))
            continue
        if identifier in target_ids:
            failed |= bool(error(f"duplicate target id {identifier}"))
        target_ids.add(identifier)

        for key in ("family", "release", "kind", "architecture", "image", "lifecycle", "adapter"):
            if not isinstance(target.get(key), str) or not target[key]:
                failed |= bool(error(f"{identifier}: {key} must be a non-empty string"))
        family = target.get("family")
        if family not in FAMILIES:
            failed |= bool(error(f"{identifier}: unknown family {family!r}"))
        else:
            found_families.add(family)
        if target.get("kind") not in {"fixed", "rolling"}:
            failed |= bool(error(f"{identifier}: kind must be fixed or rolling"))
        if target.get("status") not in STATUSES:
            failed |= bool(error(f"{identifier}: invalid status"))
        if target.get("support") not in SUPPORT:
            failed |= bool(error(f"{identifier}: invalid support state"))

        ci = target.get("ci")
        if not isinstance(ci, list) or not all(isinstance(path, str) for path in ci):
            failed |= bool(error(f"{identifier}: ci must be a list of workflow paths"))
        else:
            for path in ci:
                if not path.startswith(".github/workflows/") or not (ROOT / path).is_file():
                    failed |= bool(error(f"{identifier}: CI workflow does not exist: {path}"))
        if target.get("status") == "existing-path" and not ci:
            failed |= bool(error(f"{identifier}: existing-path requires a CI workflow"))

        gates = target.get("gates")
        if (
            not isinstance(gates, dict)
            or set(gates) != GATES
            or not all(isinstance(value, bool) for value in gates.values())
        ):
            failed |= bool(error(f"{identifier}: gates must contain each required boolean gate"))
            continue
        complete = all(gates.values())
        if target.get("support") == "supported" and not complete:
            failed |= bool(error(f"{identifier}: supported requires every gate"))
        if target.get("support") != "supported" and complete:
            failed |= bool(error(f"{identifier}: complete gates require support=supported"))
        if target.get("support") == "candidate" and not all(
            gates[gate] for gate in ("package-build", "install", "coinstall", "removal")
        ):
            failed |= bool(error(f"{identifier}: candidate requires package, install, coexistence and removal gates"))

    try:
        specification = importlib.util.spec_from_file_location("fedora_releases", ROOT / "scripts/fedora-releases.py")
        fedora_releases = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(fedora_releases)
        fedora_releases.describe(inventory)
    except (KeyError, ValueError) as exception:
        failed |= bool(error(f"Fedora releases: {exception}"))

    if CANDIDATES.is_file():
        for message in candidate_warnings(inventory, json.loads(CANDIDATES.read_text()), datetime.date.today()):
            warn(message)

    if found_families != FAMILIES:
        failed |= bool(error(f"missing requested families: {', '.join(sorted(FAMILIES - found_families))}"))
    if failed:
        return 1
    print(f"Validated {len(targets)} package targets across {len(found_families)} requested families.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
