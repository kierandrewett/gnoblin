"""Alpine repositories: reading .apk files, and writing and reading APKINDEX.tar.gz.

An .apk is two or three gzip streams back to back: an optional signature, the control part (.PKGINFO and the install
scripts) and the data part. APKINDEX.tar.gz has the same shape: a signature stream holding one member
`.SIGN.RSA256.<key file name>`, then a stream with `DESCRIPTION` and `APKINDEX`. The signature is made over the bytes of
the second stream as they are stored. The tars of the first streams have no end marker, so that a reader goes on into
the next stream, and apk finds the public key under /etc/apk/keys/<key file name>.

An index entry names a package by `Q1` plus the base64 of the SHA-1 of its control stream; apk checks a package it
downloads against that value, so a package needs no signature of its own when it is installed from a signed index.
"""

from __future__ import annotations

import base64
import gzip
import hashlib
import io
import tarfile
import zlib
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO, Protocol

INDEX_NAME = "APKINDEX.tar.gz"
SIGNATURE_SCHEMES = {"RSA": "sha1", "RSA256": "sha256"}


class AlpineError(ValueError):
    pass


class IndexSigner(Protocol):
    name: str  # the file name of the public key, as apk looks for it in /etc/apk/keys

    def sign(self, data: bytes) -> bytes:
        """An RSA PKCS#1 v1.5 signature of data with SHA-256."""
        ...


def read_streams(handle: BinaryIO, count: int) -> list[tuple[bytes, bytes]]:
    """The first `count` gzip streams of a file, as (stored bytes, decompressed bytes).

    Nothing after the last stream is read, so a package's large data part stays on disk.
    """
    streams: list[tuple[bytes, bytes]] = []
    pending = b""
    for _ in range(count):
        decoder = zlib.decompressobj(wbits=31)
        stored = bytearray()
        text = bytearray()
        data = pending
        while True:
            if not data:
                data = handle.read(1 << 16)
                if not data:
                    raise AlpineError("the file ends inside a gzip stream")
            text += decoder.decompress(data)
            if decoder.eof:
                stored += data[: len(data) - len(decoder.unused_data)]
                pending = decoder.unused_data
                break
            stored += data
            data = b""
        streams.append((bytes(stored), bytes(text)))
    return streams


def read_members(text: bytes) -> dict[str, bytes]:
    with tarfile.open(fileobj=io.BytesIO(text), mode="r:") as archive:
        result = {}
        for member in archive:
            if member.isfile():
                result[member.name] = archive.extractfile(member).read()  # type: ignore[union-attr]
        return result


@dataclass(frozen=True)
class Control:
    stored: bytes  # the control stream as the file holds it
    fields: dict[str, list[str]]  # .PKGINFO by key; a key can repeat (depend, provides)

    @property
    def checksum(self) -> str:
        return "Q1" + base64.b64encode(hashlib.sha1(self.stored).digest()).decode()


def read_control(path: Path) -> Control:
    with path.open("rb") as handle:
        # A signed package has the signature stream first; the control part is the one with .PKGINFO.
        for stored, text in read_streams(handle, 2):
            members = read_members(text)
            if ".PKGINFO" not in members:
                continue
            fields: dict[str, list[str]] = {}
            for line in members[".PKGINFO"].decode().splitlines():
                key, separator, value = line.partition(" = ")
                if separator and not key.startswith("#"):
                    fields.setdefault(key, []).append(value)
            return Control(stored, fields)
    raise AlpineError(f"{path.name} is not an Alpine package: it has no .PKGINFO")


def read_package(path: Path) -> tuple[str, str, str]:
    """(name, version, arch) of an .apk. The version is pkgver, which includes the -rN release."""
    fields = read_control(path).fields
    try:
        return fields["pkgname"][0], fields["pkgver"][0], fields["arch"][0]
    except KeyError as error:
        raise AlpineError(f"{path.name}: .PKGINFO has no {error.args[0]}") from error


def index_entry(path: Path, repository_arch: str) -> str:
    """The APKINDEX stanza of one package, with the fields and in the order apk-tools writes them.

    An architecture-independent package (arch noarch) is listed with the architecture of the repository, as abuild does
    with `apk index --rewrite-arch`. apk downloads a package from the directory of the architecture it is listed with,
    so a package listed as noarch would be looked for in a directory called noarch.
    """
    control = read_control(path)
    fields = control.fields

    def one(key: str) -> str:
        return fields[key][0] if key in fields else ""

    values = [
        ("C", control.checksum),
        ("P", one("pkgname")),
        ("V", one("pkgver")),
        ("A", repository_arch if one("arch") == "noarch" else one("arch")),
        ("S", str(path.stat().st_size)),
        ("I", one("size") or "0"),
        ("T", one("pkgdesc")),
        ("U", one("url")),
        ("L", one("license")),
        ("o", one("origin")),
        ("m", one("packager")),
        ("t", one("builddate")),
        ("c", one("commit")),
        ("D", " ".join(fields.get("depend", []))),
        ("p", " ".join(fields.get("provides", []))),
        ("i", " ".join(fields.get("install_if", []))),
        ("k", one("provider_priority")),
    ]
    return "".join(f"{key}:{value}\n" for key, value in values if value)


def tar_member(name: str, data: bytes) -> bytes:
    """One tar member, with no end marker after it."""
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = 0o644
    info.uname = info.gname = "root"
    return info.tobuf(tarfile.USTAR_FORMAT, "utf-8", "strict") + data + b"\0" * (-len(data) % 512)


def build_index(directory: Path, signer: IndexSigner, packages: list[str], description: str) -> None:
    """Write directory/APKINDEX.tar.gz for the .apk files named in packages, signed by signer.

    The directory is named after the architecture of the repository (x86_64, aarch64).
    """
    text = "\n".join(index_entry(directory / name, directory.name) for name in sorted(packages)) + "\n"
    # mtime=0 keeps the index identical when the packages are.
    index = gzip.compress(
        tar_member("DESCRIPTION", description.encode()) + tar_member("APKINDEX", text.encode()), mtime=0
    )
    signature = gzip.compress(tar_member(f".SIGN.RSA256.{signer.name}", signer.sign(index)), mtime=0)
    (directory / INDEX_NAME).write_bytes(signature + index)


@dataclass(frozen=True)
class Index:
    key_name: str
    digest: str  # sha1 or sha256: what the signature is over
    signature: bytes
    signed: bytes  # the index stream as stored, which the signature covers
    description: str
    entries: list[dict[str, str]]


def read_index(path: Path) -> Index:
    with path.open("rb") as handle:
        (_, first), (signed, second) = read_streams(handle, 2)
    members = read_members(first)
    if len(members) != 1 or not next(iter(members)).startswith(".SIGN."):
        raise AlpineError(f"{path.name}: the first stream is not a signature")
    member, signature = next(iter(members.items()))
    scheme, _, key_name = member.removeprefix(".SIGN.").partition(".")
    if scheme not in SIGNATURE_SCHEMES:
        raise AlpineError(f"{path.name}: unknown signature type {scheme}")
    parts = read_members(second)
    entries = []
    for block in parts.get("APKINDEX", b"").decode().split("\n\n"):
        if block.strip():
            entries.append(dict(line.split(":", 1) for line in block.strip().splitlines()))
    return Index(
        key_name, SIGNATURE_SCHEMES[scheme], signature, signed, parts.get("DESCRIPTION", b"").decode(), entries
    )
