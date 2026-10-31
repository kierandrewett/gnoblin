"""Verify a built tree the way a client would, before anything is published: checksums, signatures, metadata."""

from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from .packages import sha256_of
from .publish import tree_files


def _gpg_verify(home: Path, *arguments: object) -> str | None:
    result = subprocess.run(
        ["gpg", "--homedir", str(home), "--batch", "--no-tty", "--verify", *map(str, arguments)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return (
            (result.stderr or result.stdout).strip().splitlines()[-1] if (result.stderr or result.stdout) else "failed"
        )
    return None


def verify(root: Path, project: str, channel: str) -> list[str]:
    """Return a list of problems; an empty list means every check passed.

    Checks: the manifest matches the files, each RPM repository has a valid repomd.xml.asc, each APT suite has a valid
    InRelease and Release.gpg, each pacman package, database and files database has a valid detached signature, and
    gpg.key is the key the manifest names.
    """
    problems: list[str] = []
    base = root / project / channel
    manifest = json.loads((base / "manifest.json").read_text())
    home = Path(tempfile.mkdtemp(prefix="pkgrepo-verify-"))
    home.chmod(0o700)
    try:
        subprocess.run(
            ["gpg", "--homedir", str(home), "--batch", "--import", str(root / "gpg.key")],
            check=True,
            capture_output=True,
        )
        listing = subprocess.run(
            ["gpg", "--homedir", str(home), "--batch", "--with-colons", "--list-keys"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        fingerprints = {line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:")}
        if manifest["key"]["fingerprint"] not in fingerprints:
            problems.append("gpg.key does not contain the key named in manifest.json")

        for package in manifest["packages"]:
            file = base / package["path"]
            if not file.is_file():
                problems.append(f"{package['path']} is listed in the manifest but missing")
            elif sha256_of(file) != package["sha256"]:
                problems.append(f"{package['path']} does not match its sha256")

        for repomd in sorted(base.glob("*/*/*/repodata/repomd.xml")):
            error = _gpg_verify(home, repomd.with_name("repomd.xml.asc"), repomd)
            if error:
                problems.append(f"{repomd.relative_to(base)}: {error}")
        for release in sorted((base / "deb" / "dists").glob("*/Release")) if (base / "deb").is_dir() else []:
            error = _gpg_verify(home, release.with_name("InRelease"))
            if error:
                problems.append(f"{release.parent.relative_to(base)}/InRelease: {error}")
            error = _gpg_verify(home, release.with_name("Release.gpg"), release)
            if error:
                problems.append(f"{release.parent.relative_to(base)}/Release.gpg: {error}")
        for signature in sorted(base.glob("arch/*/*.sig")):
            target = signature.with_suffix("")
            if not target.exists():
                problems.append(f"{signature.relative_to(base)}: nothing to verify")
                continue
            error = _gpg_verify(home, signature, target)
            if error:
                problems.append(f"{signature.relative_to(base)}: {error}")
        rpms = [r for r in tree_files(root, f"{project}/{channel}") if r.endswith(".rpm")]
        if rpms:
            database = home / "rpmdb"
            subprocess.run(
                ["rpmkeys", "--dbpath", str(database), "--import", str(root / "gpg.key")],
                check=True,
                capture_output=True,
            )
            for relative in rpms:
                result = subprocess.run(
                    ["rpmkeys", "--dbpath", str(database), "-K", str(root / relative)], capture_output=True, text=True
                )
                if result.returncode != 0 or "signatures OK" not in result.stdout:
                    problems.append(
                        f"{relative}: RPM signature check failed: {(result.stdout + result.stderr).strip()}"
                    )
    finally:
        shutil.rmtree(home, ignore_errors=True)
    return problems
