#!/usr/bin/env python3
"""Add release packages to a signed, distro-specific APT archive."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import shutil
import subprocess
from dataclasses import dataclass
from datetime import UTC, datetime
from email.utils import format_datetime
from pathlib import Path


@dataclass(frozen=True)
class Suite:
    archive: str
    suite: str
    asset: str
    version_suffix: str
    description: str


SUITES = (
    Suite("debian", "13", "gnoblin-debian13-amd64.deb", "debian13", "Gnoblin packages for Debian 13"),
    Suite("ubuntu", "24.04", "gnoblin-ubuntu24.04-amd64.deb", "ubuntu24.04", "Gnoblin packages for Ubuntu 24.04 LTS"),
    Suite("ubuntu", "26.04", "gnoblin-ubuntu26.04-amd64.deb", "ubuntu26.04", "Gnoblin packages for Ubuntu 26.04 LTS"),
)


def control_fields(package: Path) -> dict[str, str]:
    output = subprocess.check_output(["dpkg-deb", "-f", package], text=True)
    fields: dict[str, str] = {}
    current: str | None = None
    for line in output.splitlines():
        if line.startswith((" ", "\t")):
            if current is None:
                raise ValueError(f"invalid control data in {package}")
            fields[current] += "\n" + line
            continue
        key, separator, value = line.partition(":")
        if not separator:
            raise ValueError(f"invalid control data in {package}: {line}")
        current = key
        fields[key] = value.lstrip()
    return fields


def digest(path: Path, algorithm: str) -> str:
    hasher = hashlib.new(algorithm)
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def package_stanza(package: Path, relative_name: Path) -> str:
    fields = control_fields(package)
    required = ("Package", "Version", "Architecture")
    missing = [field for field in required if not fields.get(field)]
    if missing:
        raise ValueError(f"{package} is missing {', '.join(missing)}")
    control = subprocess.check_output(["dpkg-deb", "-f", package], text=True).strip()
    return "\n".join(
        (
            control,
            f"Filename: {relative_name.as_posix()}",
            f"Size: {package.stat().st_size}",
            f"MD5sum: {digest(package, 'md5')}",
            f"SHA256: {digest(package, 'sha256')}",
            "",
        )
    )


def write_packages(archive_root: Path, suite: Suite) -> list[Path]:
    pool = archive_root / suite.archive / "pool" / "main" / "g" / "gnoblin"
    packages = [
        package
        for package in sorted(pool.glob("*.deb"))
        if control_fields(package).get("Version", "").endswith(f"~{suite.version_suffix}")
    ]
    if not packages:
        raise ValueError(f"{suite.archive} {suite.suite} has no packages")
    index = archive_root / suite.archive / "dists" / suite.suite / "main" / "binary-amd64" / "Packages"
    index.parent.mkdir(parents=True, exist_ok=True)
    body = "\n".join(package_stanza(package, package.relative_to(archive_root / suite.archive)) for package in packages)
    index.write_text(body, encoding="utf-8")
    compressed = index.with_suffix(".gz")
    with compressed.open("wb") as file:
        with gzip.GzipFile(filename="", mode="wb", fileobj=file, mtime=0) as stream:
            stream.write(body.encode())
    return [index, compressed]


def write_release(archive_root: Path, suite: Suite, indexes: list[Path]) -> Path:
    root = archive_root / suite.archive
    release = root / "dists" / suite.suite / "Release"
    now = format_datetime(datetime.now(UTC), usegmt=True)
    lines = [
        "Origin: Gnoblin",
        "Label: Gnoblin",
        f"Suite: {suite.suite}",
        f"Codename: {suite.suite}",
        f"Date: {now}",
        "Architectures: amd64",
        "Components: main",
        f"Description: {suite.description}",
        "SHA256:",
    ]
    for index in indexes:
        relative = index.relative_to(release.parent)
        lines.append(f" {digest(index, 'sha256')} {index.stat().st_size:16d} {relative.as_posix()}")
    release.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return release


def sign(release: Path, key: str, passphrase: str) -> None:
    command = [
        "gpg",
        "--batch",
        "--yes",
        "--pinentry-mode",
        "loopback",
        "--passphrase",
        passphrase,
        "--local-user",
        key,
    ]
    subprocess.run(
        [*command, "--armor", "--detach-sign", "--output", release.with_name("Release.gpg"), release], check=True
    )
    subprocess.run([*command, "--clearsign", "--output", release.with_name("InRelease"), release], check=True)


def add_package(archive_root: Path, downloads: Path, suite: Suite) -> None:
    source = downloads / suite.asset
    if not source.is_file():
        raise ValueError(f"missing release asset: {source}")
    fields = control_fields(source)
    if fields.get("Package") != "gnoblin" or fields.get("Architecture") != "amd64":
        raise ValueError(f"unexpected package metadata in {source}")
    version = fields.get("Version", "")
    if not version.endswith(f"~{suite.version_suffix}"):
        raise ValueError(f"{source} is not built for {suite.archive} {suite.suite}: {version}")
    destination = archive_root / suite.archive / "pool" / "main" / "g" / "gnoblin" / f"gnoblin_{version}_amd64.deb"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and digest(destination, "sha256") != digest(source, "sha256"):
        raise ValueError(f"refusing to replace published package {destination}")
    if not destination.exists():
        shutil.copy2(source, destination)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="directory containing apt/debian and apt/ubuntu")
    parser.add_argument("--downloads", type=Path, required=True, help="directory containing release .deb files")
    parser.add_argument("--keyring", type=Path, required=True, help="public archive key to publish")
    parser.add_argument("--signing-key", help="GPG fingerprint used to sign InRelease")
    parser.add_argument("--passphrase-file", type=Path, help="file containing the signing-key passphrase")
    parser.add_argument("--unsigned", action="store_true", help="write indexes without GPG signatures")
    args = parser.parse_args()

    if args.unsigned == bool(args.signing_key):
        parser.error("pass exactly one of --unsigned or --signing-key")
    if args.unsigned and args.passphrase_file:
        parser.error("--passphrase-file requires --signing-key")
    if args.signing_key and not args.passphrase_file:
        parser.error("--passphrase-file is required when signing")
    if not args.downloads.is_dir():
        parser.error(f"downloads directory does not exist: {args.downloads}")
    if not args.keyring.is_file():
        parser.error(f"archive key does not exist: {args.keyring}")

    args.archive.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.keyring, args.archive / "gnoblin-archive-keyring.asc")
    passphrase = args.passphrase_file.read_text(encoding="utf-8").rstrip("\n") if args.passphrase_file else ""
    if args.signing_key and not passphrase:
        parser.error("signing passphrase is empty")
    for suite in SUITES:
        add_package(args.archive, args.downloads, suite)
        indexes = write_packages(args.archive, suite)
        release = write_release(args.archive, suite, indexes)
        if args.signing_key:
            sign(release, args.signing_key, passphrase)


if __name__ == "__main__":
    main()
