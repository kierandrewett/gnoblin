"""The three repository formats: RPM (createrepo_c), APT (apt-ftparchive) and pacman (repo-add).

Each builder regenerates the metadata of one directory from the package files in it, then signs the metadata. Nothing
here keeps state between runs: the package files are the state.
"""

from __future__ import annotations

import gzip
import lzma
import shutil
from pathlib import Path

from .runner import require, run
from .signing import Signer


# --- RPM -------------------------------------------------------------------------------------------------------------


def build_rpm(directory: Path, signer: Signer, new_files: set[str]) -> None:
    """Sign the new RPMs, write repodata/ and sign repodata/repomd.xml (repomd.xml.asc, which dnf's repo_gpgcheck reads).

    No sqlite databases: dnf and zypper read the XML, and the databases double the metadata for no benefit.
    """
    require("createrepo_c", "rpmsign", "gpg")
    for name in sorted(new_files):
        signer.sign_rpm(directory / name)
    shutil.rmtree(directory / "repodata", ignore_errors=True)
    run(["createrepo_c", "--quiet", "--no-database", "--general-compress-type", "gz", directory], capture=True)
    repomd = directory / "repodata" / "repomd.xml"
    signer.detach(repomd, repomd.with_name("repomd.xml.asc"), armor=True)


# --- APT -------------------------------------------------------------------------------------------------------------


def _stanzas(text: str) -> list[str]:
    return [block for block in text.split("\n\n") if block.strip()]


def _architecture(stanza: str) -> str:
    for line in stanza.splitlines():
        if line.startswith("Architecture:"):
            return line.split(":", 1)[1].strip()
    raise ValueError("a Packages stanza has no Architecture field")


def build_apt_suite(
    root: Path,
    suite: str,
    architectures: list[str],
    *,
    origin: str,
    label: str,
    description: str,
    signer: Signer,
) -> None:
    """Write dists/<suite>/ for the packages in root/pool/<suite>, one component (main), and sign its Release file."""
    require("apt-ftparchive", "gpg")
    packages_all = run(["apt-ftparchive", "packages", f"pool/{suite}"], cwd=root, capture=True, quiet=True)
    stanzas = _stanzas(packages_all)
    dist = root / "dists" / suite
    shutil.rmtree(dist, ignore_errors=True)
    for architecture in architectures:
        index_dir = dist / "main" / f"binary-{architecture}"
        index_dir.mkdir(parents=True)
        chosen = [s for s in stanzas if _architecture(s) in (architecture, "all")]
        body = ("\n\n".join(chosen) + "\n\n").encode() if chosen else b""
        (index_dir / "Packages").write_bytes(body)
        # mtime=0 keeps the compressed files identical when the packages are.
        (index_dir / "Packages.gz").write_bytes(gzip.compress(body, mtime=0))
        (index_dir / "Packages.xz").write_bytes(lzma.compress(body))
    options = {
        "Origin": origin,
        "Label": label,
        "Suite": suite,
        "Codename": suite,
        "Architectures": " ".join(architectures),
        "Components": "main",
        "Description": description,
    }
    arguments: list[object] = ["apt-ftparchive"]
    for key, value in options.items():
        arguments += ["-o", f"APT::FTPArchive::Release::{key}={value}"]
    arguments += ["release", f"dists/{suite}"]
    release = run(arguments, cwd=root, capture=True, quiet=True)
    (dist / "Release").write_text(release)
    signer.clearsign(dist / "Release", dist / "InRelease")
    signer.detach(dist / "Release", dist / "Release.gpg", armor=True)


# --- pacman ----------------------------------------------------------------------------------------------------------


def build_pacman(directory: Path, repo_name: str, signer: Signer, packages: list[str]) -> None:
    """Sign packages that have no signature yet, write <repo_name>.db and .files, and sign both.

    pacman fetches <repo_name>.db and <repo_name>.db.sig, so the names carry the section name of pacman.conf. The
    files are copies, not the symlinks repo-add makes: static hosts such as GitHub Pages and S3 have no symlinks.
    """
    require("repo-add", "gpg", "zstd")
    for name in packages:
        signature = directory / f"{name}.sig"
        if not signature.exists():
            signer.detach(directory / name, signature, armor=False)
    for stale in directory.glob(f"{repo_name}.*"):
        stale.unlink()
    database = directory / f"{repo_name}.db.tar.zst"
    run(["repo-add", "--nocolor", database, *[directory / name for name in sorted(packages)]], capture=True)
    for kind in ("db", "files"):
        archive = directory / f"{repo_name}.{kind}.tar.zst"
        link = directory / f"{repo_name}.{kind}"
        if link.exists() or link.is_symlink():
            link.unlink()
        shutil.copyfile(archive, link)
        signer.detach(archive, directory / f"{repo_name}.{kind}.tar.zst.sig", armor=False)
        shutil.copyfile(directory / f"{repo_name}.{kind}.tar.zst.sig", directory / f"{repo_name}.{kind}.sig")
    for old in directory.glob(f"{repo_name}.*.old*"):
        old.unlink()
