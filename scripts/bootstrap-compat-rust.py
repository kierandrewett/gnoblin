#!/usr/bin/env python3
"""Install the pinned build-only Rust toolchain for old DEB compatibility builds."""

import argparse
import hashlib
import platform
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request


VERSION = "1.85.1"
TARGET = "x86_64-unknown-linux-gnu"
# Values are from https://static.rust-lang.org/dist/channel-rust-1.85.1.toml.
COMPONENTS = (
    (
        "rustc",
        "https://static.rust-lang.org/dist/2025-03-18/rustc-1.85.1-x86_64-unknown-linux-gnu.tar.gz",
        "2c7c89df662fc62bde0b75e60602ca2c64906edbecd79fd95745dfc635b05479",
    ),
    (
        "cargo",
        "https://static.rust-lang.org/dist/2025-03-18/cargo-1.85.1-x86_64-unknown-linux-gnu.tar.gz",
        "1646065e914ff61d2073aa0a9da8fafe31544dc7234a378b60f7b5aa5aebb22b",
    ),
    (
        "rust-std",
        "https://static.rust-lang.org/dist/2025-03-18/rust-std-1.85.1-x86_64-unknown-linux-gnu.tar.gz",
        "3e14ec3f9f622f2b577817f71413259a401c0b233a527ef561342f3c10a777a6",
    ),
)


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked_archive(name, url, expected, downloads):
    archive = downloads / (name + ".tar.gz")
    if not archive.exists():
        partial = archive.with_suffix(".part")
        with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as target:
            shutil.copyfileobj(response, target)
        partial.replace(archive)
    actual = sha256_file(archive)
    if actual != expected:
        raise RuntimeError(f"Rust {name} checksum mismatch: {archive}")
    return archive


def extract(archive, destination):
    root = destination.resolve()
    with tarfile.open(archive) as source:
        for member in source.getmembers():
            member_path = destination / member.name
            if Path(member.name).is_absolute() or not member_path.resolve().is_relative_to(root):
                raise RuntimeError(f"Rust archive member escapes staging: {member.name}")
            if member.ischr() or member.isblk() or member.isfifo():
                raise RuntimeError(f"Rust archive member has unsupported type: {member.name}")
            if member.issym():
                link_path = member_path.parent / member.linkname
            elif member.islnk():
                link_path = destination / member.linkname
            else:
                continue
            if Path(member.linkname).is_absolute() or not link_path.resolve().is_relative_to(root):
                raise RuntimeError(f"Rust archive link escapes staging: {member.name}")
        source.extractall(destination)


def install_component(name, url, digest, prefix, downloads):
    archive = checked_archive(name, url, digest, downloads)
    with tempfile.TemporaryDirectory(prefix="gnoblin-rust-", dir=downloads) as temporary:
        staging = Path(temporary)
        extract(archive, staging)
        children = list(staging.iterdir())
        if len(children) != 1:
            raise RuntimeError(f"Unexpected Rust archive layout: {archive}")
        installer = children[0] / "install.sh"
        if not installer.is_file():
            raise RuntimeError(f"Rust component has no installer: {archive}")
        subprocess.run([str(installer), f"--prefix={prefix}", "--disable-ldconfig"], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, required=True)
    args = parser.parse_args()
    if platform.machine() != "x86_64" or platform.system() != "Linux":
        parser.error(f"only {TARGET} is pinned for this experimental bootstrap")
    prefix = args.prefix.resolve()
    downloads = prefix / "rust-downloads" / VERSION
    downloads.mkdir(parents=True, exist_ok=True)
    for component in COMPONENTS:
        install_component(*component, prefix, downloads)
    rustc = prefix / "bin/rustc"
    cargo = prefix / "bin/cargo"
    for program in (rustc, cargo):
        if not program.is_file():
            raise RuntimeError(f"Rust bootstrap did not install {program}")
    version = subprocess.check_output([str(rustc), "--version"], text=True).split()[1]
    if version != VERSION:
        raise RuntimeError(f"Rust bootstrap produced {version}, expected {VERSION}")
    print(f"Installed build-only Rust {VERSION} for {TARGET} at {prefix}")


if __name__ == "__main__":
    main()
