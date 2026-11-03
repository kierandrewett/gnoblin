#!/usr/bin/env python3
"""The host side of the package suite, for scripts/run-package-tests.sh.

usage: suite.py target TARGETS_JSON SLUG ARCH            print the matrix entry of one target as JSON
       suite.py select TARGET_JSON ARTIFACT_DIR OUT_DIR  copy the packages of the target into OUT_DIR
       suite.py published TARGET_JSON TARGETS_JSON OUT_DIR
                                                         download the packages that repo.gnoblin.org publishes for the
                                                         target, one folder for each channel, or write OUT_DIR/SKIP
       suite.py layer NAME RESULTS_TSV REPORT_DIR STARTED  write the result of a shell layer
       suite.py finish TARGET_JSON REPORT_DIR RELEASE_TAG  write report.json, junit.xml and summary.md
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import report  # noqa: E402
import targets  # noqa: E402

REPOSITORY = os.environ.get("GNOBLIN_REPO_URL", "https://repo.gnoblin.org").rstrip("/")
USER_AGENT = "gnoblin-package-tests (+https://github.com/kierandrewett/gnoblin)"
# The format names of the repository manifest (scripts/pkgrepo).
MANIFEST_FORMAT = {"rpm": "rpm", "deb": "deb", "archlinux": "arch", "apk": "apk"}


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def claimed_releases(matrix: list[dict], fmt: str) -> set[str]:
    """The releases that a target with its own package lane names: its packages are never the shared ones."""
    found = set()
    for entry in matrix:
        if targets.own_lane(entry) and targets.package_format(entry) == fmt:
            found.update({entry["release"], entry["slug"]})
    return found


def published(target: dict, matrix: list[dict], out: Path) -> int:
    """The newest published version of each Gnoblin package for the family, release and architecture of the target.

    A target with its own lane takes the packages of its release. A target of the shared set takes those of a release
    that no lane of its own claims (the shared deb suite, Arch, the Tumbleweed rpm)."""
    out.mkdir(parents=True, exist_ok=True)
    fmt = targets.package_format(target)
    arch_names = {targets.EXPECTATIONS["architectures"][target["arch"]][fmt], targets.EXPECTATIONS["independent"][fmt]}
    claimed = claimed_releases(matrix, fmt)
    found_any = False
    notes = []
    for channel in ("stable", "nightly"):
        try:
            manifest = json.loads(fetch(f"{REPOSITORY}/gnoblin/{channel}/manifest.json"))
        except (urllib.error.URLError, OSError, ValueError) as error:
            notes.append(f"{channel}: {REPOSITORY} could not be read ({error})")
            continue
        rows = []
        for item in manifest.get("packages", []):
            if item.get("format") != MANIFEST_FORMAT[fmt] or item.get("arch") not in arch_names:
                continue
            if item.get("name") not in targets.EXPECTATIONS["packages"]:
                continue
            release = item.get("release", "")
            if targets.own_lane(target):
                if release not in (target["release"], target["slug"]):
                    continue
            elif release in claimed:
                continue
            rows.append(item)
        if not rows:
            notes.append(f"{channel}: no {MANIFEST_FORMAT[fmt]} packages for this release and architecture")
            continue
        newest: dict[str, dict] = {}
        for item in rows:
            # The manifest lists the versions of a package oldest first within one build; keep the latest entry.
            newest[item["name"]] = item
        directory = out / channel
        directory.mkdir(exist_ok=True)
        try:
            downloads = {
                item["filename"]: fetch(f"{REPOSITORY}/gnoblin/{channel}/{item['path']}") for item in newest.values()
            }
        except (urllib.error.URLError, OSError) as error:
            notes.append(f"{channel}: a package of the manifest could not be downloaded ({error})")
            continue
        for item in newest.values():
            data = downloads[item["filename"]]
            if hashlib.sha256(data).hexdigest() != item["sha256"]:
                raise SystemExit(f"{item['path']}: the SHA-256 does not match the manifest of {channel}")
            (directory / item["filename"]).write_bytes(data)
            print(f"published {channel}: {item['filename']} {item['version']}")
        found_any = True
    if not found_any:
        reason = (
            f"no published tree for {target['name']} ({MANIFEST_FORMAT[fmt]}, {target['arch']}) at {REPOSITORY}: "
            + "; ".join(notes)
        )
        (out / "SKIP").write_text(reason + "\n")
        print(reason)
    return 0


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    command, args = sys.argv[1], sys.argv[2:]
    if command == "target":
        data = json.loads(Path(args[0]).read_text())
        print(json.dumps(targets.find(data["matrix"], args[1], args[2])))
        return 0
    if command == "select":
        target = json.loads(Path(args[0]).read_text())
        out = Path(args[2])
        out.mkdir(parents=True, exist_ok=True)
        files = targets.select(target, Path(args[1]))
        for path in files:
            shutil.copy2(path, out / path.name)
            print(path.name)
        return 0 if files else 1
    if command == "published":
        target = json.loads(Path(args[0]).read_text())
        matrix = json.loads(Path(args[1]).read_text())["matrix"]
        return published(target, matrix, Path(args[2]))
    if command == "layer":
        result = report.from_tsv(args[0], Path(args[1]), float(args[3]), time.time())
        report.write_layer(Path(args[2]), result)
        print(f"[{args[0]}] {result['status']}: {len(result['cases'])} cases")
        return 0
    if command == "finish":
        target = json.loads(Path(args[0]).read_text())
        result = report.finish(Path(args[1]), target, args[2])
        print((Path(args[1]) / "summary.md").read_text())
        return 0 if result["result"] == report.PASS else 1
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
