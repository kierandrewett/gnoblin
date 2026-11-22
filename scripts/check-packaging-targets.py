#!/usr/bin/env python3
"""Validate the canonical Gnoblin package target inventory.

Run this after changing packaging/targets.json. The inventory records desired
targets and the evidence that exists today; an image or a CI path never by
itself marks a target as supported.
"""

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent.parent
INVENTORY = ROOT / "packaging/targets.json"
FAMILIES = {"fedora", "el", "debian", "ubuntu", "arch", "opensuse-leap", "opensuse-tumbleweed", "nixos"}
STATUSES = {"existing-path", "probe-needed", "planned"}
SUPPORT = {"unsupported", "candidate", "supported"}
GATES = {"dependency-probe", "package-build", "install", "coinstall", "graphical-session", "removal"}


def error(message):
    print(f"packaging target inventory: {message}", file=sys.stderr)
    return 1


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

    if found_families != FAMILIES:
        failed |= bool(error(f"missing requested families: {', '.join(sorted(FAMILIES - found_families))}"))
    if failed:
        return 1
    print(f"Validated {len(targets)} package targets across {len(found_families)} requested families.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
