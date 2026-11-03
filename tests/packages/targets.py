"""Which package files belong to which target.

A target is one entry of the matrix that scripts/supported-targets.py writes: one release (or rolling system) on one
architecture. Its packages are in the artifact <install_artifact>-<release tag><install_suffix>-<arch>. A target with
a package lane of its own installs every file of its format in its own artifact. A target that installs the shared
packages takes the files of its format that carry its release (the .tumbleweed rpm) or no release (the deb), and a
glibc target of the shared set takes the tar packages too.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from formats import format_of

HERE = Path(__file__).resolve().parent
EXPECTATIONS = json.loads((HERE / "expectations.json").read_text())


def package_format(target: dict) -> str:
    """The pkgsmith format of the kind of a target: rpm, deb, archlinux or apk."""
    return EXPECTATIONS["formats"][target["kind"]]


def own_lane(target: dict) -> bool:
    return bool(target.get("lane"))


def takes_tar(target: dict) -> bool:
    """The glibc targets of the shared set check the tar packages too. The tar packages come from the glibc prefix."""
    return not own_lane(target) and package_format(target) != "apk"


def formats(target: dict) -> list[str]:
    return [package_format(target), *(["tar"] if takes_tar(target) else [])]


def artifact_name(target: dict, release_tag: str) -> str:
    return f"{target['install_artifact']}-{release_tag}{target['install_suffix']}-{target['arch']}"


def selects(target: dict, filename: str) -> bool:
    """True when the file in the artifact of the target is a package that the target installs and tests."""
    fmt = format_of(filename)
    if fmt is None or fmt not in formats(target):
        return False
    slug = target["slug"]
    if fmt == "rpm":
        return re.search(rf"\.{re.escape(slug)}\.[a-z0-9_]+\.rpm$", filename) is not None
    if fmt == "deb":
        revision_has_release = re.search(r"-\d+~[^_]+_[a-z0-9]+\.deb$", filename) is not None
        if own_lane(target):
            return filename.endswith(tuple(f"~{slug}_{arch}.deb" for arch in deb_architectures(target)))
        return not revision_has_release
    return True


def deb_architectures(target: dict) -> tuple[str, str]:
    return EXPECTATIONS["architectures"][target["arch"]]["deb"], EXPECTATIONS["independent"]["deb"]


def select(target: dict, directory: Path) -> list[Path]:
    return sorted(path for path in directory.iterdir() if path.is_file() and selects(target, path.name))


def find(matrix: list[dict], slug: str, arch: str) -> dict:
    found = [entry for entry in matrix if entry["slug"] == slug and entry["arch"] == arch]
    if len(found) != 1:
        raise SystemExit(f"no single target {slug} {arch} in the target list ({len(found)} found)")
    return found[0]
