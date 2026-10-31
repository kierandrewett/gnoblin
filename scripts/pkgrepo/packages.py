"""Package records, and reading the metadata of RPM, Debian and Arch package files."""

from __future__ import annotations

import hashlib
import re
from dataclasses import asdict, dataclass
from pathlib import Path

from .runner import run

ARCH_INDEPENDENT = {"rpm": "noarch", "deb": "all", "arch": "any"}
FAMILIES = {"fedora": "rpm", "opensuse": "rpm", "deb": "deb", "arch": "arch"}


@dataclass(frozen=True)
class Package:
    """One package file and the place it has in the repository tree."""

    format: str  # rpm, deb or arch
    family: str  # fedora, opensuse, deb or arch
    release: str  # Fedora release, openSUSE name, Debian suite; empty for Arch
    name: str
    version: str  # the full version in the format's own notation: [epoch:]version-release
    arch: str  # the package's own architecture word: x86_64, noarch, amd64, all, any
    filename: str
    size: int
    sha256: str

    @property
    def independent(self) -> bool:
        return self.arch == ARCH_INDEPENDENT[self.format]

    @property
    def group(self) -> tuple[str, str, str, str, str]:
        """Packages of one group replace each other when their versions differ."""
        return (self.family, self.release, self.name, self.arch, self.format)

    def as_dict(self) -> dict[str, object]:
        return asdict(self)


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_rpm(path: Path) -> tuple[str, str, str]:
    """Return (name, version, arch) of an RPM; the version is epoch:version-release when an epoch is set."""
    fields = run(
        ["rpm", "-qp", "--nosignature", "--queryformat", r"%{NAME}\n%{EPOCH}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n", path],
        capture=True,
        quiet=True,
    ).split("\n")
    name, epoch, version, release, arch = fields[:5]
    full = f"{version}-{release}"
    if epoch not in ("", "(none)"):
        full = f"{epoch}:{full}"
    return name, full, arch


def read_deb(path: Path) -> tuple[str, str, str]:
    out = run(["dpkg-deb", "-f", path, "Package", "Version", "Architecture"], capture=True, quiet=True)
    fields = dict(line.split(": ", 1) for line in out.splitlines() if ": " in line)
    return fields["Package"], fields["Version"], fields["Architecture"]


_ARCH_FILE = re.compile(
    r"^(?P<name>.+)-(?P<pkgver>[^-]+)-(?P<pkgrel>[^-]+)-(?P<arch>[^-]+)\.pkg\.tar\.(zst|xz|gz|bz2)$"
)


def read_arch(path: Path) -> tuple[str, str, str]:
    """Return (name, version, arch) from a pacman package. The file name carries the three values; .PKGINFO must agree."""
    match = _ARCH_FILE.match(path.name)
    if not match:
        raise ValueError(f"{path.name} is not named name-pkgver-pkgrel-arch.pkg.tar.*")
    info = run(["tar", "-xOf", path, ".PKGINFO"], capture=True, quiet=True)
    values = {}
    for line in info.splitlines():
        key, sep, value = line.partition(" = ")
        if sep and key in ("pkgname", "pkgver", "arch"):
            values[key] = value
    if (values.get("pkgname"), values.get("pkgver"), values.get("arch")) != (
        match["name"],
        f"{match['pkgver']}-{match['pkgrel']}",
        match["arch"],
    ):
        raise ValueError(f"{path.name}: the file name and .PKGINFO disagree ({values})")
    return match["name"], values["pkgver"], values["arch"]


READERS = {"rpm": read_rpm, "deb": read_deb, "arch": read_arch}
SUFFIXES = {"rpm": ".rpm", "deb": ".deb", "arch": ".pkg.tar.zst"}


def scan(input_root: Path) -> list[tuple[Package, Path]]:
    """Read every package under input_root/<family>/<release>/. Arch packages live under input_root/arch/.

    Debug packages and source packages are not part of a repository and are skipped, with a note.
    """
    found: list[tuple[Package, Path]] = []
    for family, package_format in FAMILIES.items():
        base = input_root / family
        if not base.is_dir():
            continue
        suffix = SUFFIXES[package_format]
        for path in sorted(base.rglob("*")):
            if not path.is_file() or not path.name.endswith(suffix):
                continue
            relative = path.relative_to(base).parts
            if package_format == "arch":
                release = ""
            elif len(relative) < 2:
                raise ValueError(
                    f"{path}: {family} packages belong in {input_root}/{family}/<release>/ "
                    f"({'the Debian suite' if family == 'deb' else 'the distribution release'})"
                )
            else:
                release = relative[0]
            if path.name.endswith(".src.rpm"):
                continue
            name, version, arch = READERS[package_format](path)
            if re.search(r"-(debuginfo|debugsource)$|-dbgsym$|-debug$", name):
                print(f"skipping {path.name}: debug packages are not published", flush=True)
                continue
            found.append(
                (
                    Package(
                        format=package_format,
                        family=family,
                        release=release,
                        name=name,
                        version=version,
                        arch=arch,
                        filename=path.name,
                        size=path.stat().st_size,
                        sha256=sha256_of(path),
                    ),
                    path,
                )
            )
    return found
