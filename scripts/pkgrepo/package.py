"""Build packages from the nfpm.yaml files that sit beside the code they package.

A package is a folder with an nfpm.yaml in it. `find_manifests` walks a source tree and returns every one. `build`
runs nfpm once per package and format, with the manifest's folder as the working directory, so a manifest names its
scripts relative to itself. The manifest reads its files from ${STAGE}, a staged prefix laid out like the installed
system (usr/bin/...), for example the DESTDIR of `make install`.

nfpm does not look at the binaries it packages, so it adds no shared-library dependencies. A manifest lists them in
`depends`. The line `- "@scan"` in `depends` adds what the packaged binaries need: `scan_dependencies` reads the
NEEDED libraries of every ELF file the manifest packages and names the distro package that owns each one, so it must
run on the distro the format is for (a Fedora container for rpm, an Arch container for archlinux). It must set `version_schema: none`, `version: ${VERSION}` and `release: 1`. It may write a distro library as `@name`, or `@name >= 1.4`, which `resolve` replaces with that distro's
package name from a dependency table, in that format's version syntax.
"""

from __future__ import annotations

import glob
import json
import os
import re
import tempfile
from pathlib import Path
from typing import Mapping, Sequence

from .runner import CommandError, run
from .versions import semver_to_arch, semver_to_rpm

FORMATS = ("rpm", "deb", "archlinux")
SKIP_DIRECTORIES = {
    ".git",
    "vendor",
    "target",
    "build",
    "node_modules",
    "subprojects",
    "quickshell",
}
BASE_TABLE = Path(__file__).with_name("dependencies.json")

_REFERENCE = re.compile(
    r"""^(?P<indent>\s*-\s*)(?P<quote>["']?)@(?P<name>[A-Za-z0-9_.+-]+)"""
    r"""(?:\s*(?P<op><=|>=|<|>|=)\s*(?P<version>[^\s"']+))?(?P=quote)\s*$"""
)
_DEB_OPERATORS = {"<": "<<", ">": ">>"}


def find_manifests(root: Path) -> list[Path]:
    """Every nfpm.yaml below root, in a stable order, without descending into build output or vendored code."""
    found: list[Path] = []
    for directory, names, files in os.walk(root):
        names[:] = sorted(name for name in names if name not in SKIP_DIRECTORIES)
        if "nfpm.yaml" in files:
            found.append(Path(directory) / "nfpm.yaml")
    return found


def package_name(manifest: Path) -> str:
    for line in manifest.read_text().splitlines():
        if line.startswith("name:"):
            return line.split(":", 1)[1].strip()
    raise CommandError(f"{manifest}: no top-level name")


def load_table(*paths: Path) -> dict[str, dict[str, str]]:
    """The dependency table: the shared base, then each project file on top of it."""
    table: dict[str, dict[str, str]] = {}
    for path in (BASE_TABLE, *paths):
        if not path.is_file():
            continue
        for name, formats in json.loads(path.read_text()).items():
            table.setdefault(name, {}).update(formats)
    return table


def format_version(semver: str, fmt: str) -> str:
    """The version string of format fmt. The manifest sets `version_schema: none`, so nfpm writes it as given and the
    ordering rules stay the ones of versions.py, which the repository tool tests."""
    return semver_to_arch(semver) if fmt == "archlinux" else semver_to_rpm(semver)


def _render(name: str, operator: str | None, version: str | None, fmt: str) -> str:
    if operator is None:
        return name
    if fmt == "deb":
        return f"{name} ({_DEB_OPERATORS.get(operator, operator)} {version})"
    if fmt == "archlinux":
        return f"{name}{operator}{version}"
    return f"{name} {operator} {version}"


def resolve(
    text: str,
    fmt: str,
    table: Mapping[str, Mapping[str, str]],
    source: str = "manifest",
    scanned: Sequence[str] = (),
) -> str:
    """Replace each `- @name [op version]` list entry with the package name of format fmt, and `- @scan` with the
    packages that own the libraries the binaries need."""
    out: list[str] = []
    for number, line in enumerate(text.splitlines(), 1):
        match = _REFERENCE.match(line)
        if match is None:
            out.append(line)
            continue
        name = match["name"]
        if name == "scan":
            out.extend(f"{match['indent']}{package}" for package in scanned)
            continue
        package = table.get(name, {}).get(fmt)
        if package is None:
            raise CommandError(f"{source}:{number}: no {fmt} package for @{name} in the dependency table")
        out.append(f"{match['indent']}{_render(package, match['op'], match['version'], fmt)}")
    return "\n".join(out) + "\n"


def packaged_files(text: str, stage: Path) -> list[Path]:
    """The regular files the manifest takes from the stage: every `src:` below ${STAGE}, a folder walked in full."""
    found: list[Path] = []
    for line in text.splitlines():
        match = re.match(r"""^\s*(?:-\s*)?src:\s*["']?([^"'#]+?)["']?\s*$""", line)
        if match is None or "${STAGE}" not in match[1]:
            continue
        pattern = match[1].replace("${STAGE}", str(stage))
        for path in sorted(glob.glob(pattern)):
            location = Path(path)
            if location.is_dir():
                found.extend(sorted(p for p in location.rglob("*") if p.is_file() and not p.is_symlink()))
            elif location.is_file() and not location.is_symlink():
                found.append(location)
    return found


def _is_elf(path: Path) -> bool:
    with path.open("rb") as handle:
        return handle.read(4) == b"\x7fELF"


def needed_libraries(files: Sequence[Path]) -> set[str]:
    """The NEEDED sonames of every ELF file, minus the sonames the same files provide."""
    needed: set[str] = set()
    provided: set[str] = set()
    for path in files:
        if not _is_elf(path):
            continue
        provided.add(path.name)
        for line in run(["readelf", "-d", path], capture=True, quiet=True).splitlines():
            if "(NEEDED)" in line and (found := re.search(r"\[([^\]]+)\]", line)):
                needed.add(found[1])
            elif "(SONAME)" in line and (found := re.search(r"\[([^\]]+)\]", line)):
                provided.add(found[1])
    return needed - provided


def _library_paths() -> dict[str, list[str]]:
    """soname -> installed paths, from the loader cache of this machine."""
    paths: dict[str, list[str]] = {}
    for line in run(["ldconfig", "-p"], capture=True, quiet=True).splitlines():
        if "=>" in line:
            name, _, path = line.partition("=>")
            paths.setdefault(name.split()[0], []).append(path.strip())
    return paths


_OWNER_COMMANDS = {
    "rpm": lambda path: ["rpm", "-qf", "--qf", "%{NAME}\\n", path],
    "deb": lambda path: ["dpkg-query", "-S", path],
    "archlinux": lambda path: ["pacman", "-Qoq", path],
}


def owner_of(path: str, fmt: str) -> str:
    """The name of the installed package that owns path."""
    first = run(_OWNER_COMMANDS[fmt](path), capture=True, quiet=True).splitlines()[0].strip()
    return first.split(":")[0] if fmt == "deb" else first


def scan_dependencies(files: Sequence[Path], fmt: str, ignore: Sequence[str] = ()) -> list[str]:
    """Names of the distro packages that own the libraries the files need, sorted, without duplicates.

    A library that no installed package owns stops the build: the package would install and then fail to start.
    ignore names packages that must not be listed, for example the one being built."""
    installed = _library_paths()
    owners: set[str] = set()
    for soname in sorted(needed_libraries(files)):
        candidates = installed.get(soname)
        if not candidates:
            raise CommandError(f"no installed library provides {soname}; install its package in the build environment")
        owners.add(owner_of(candidates[0], fmt))
    return sorted(owners - set(ignore))


def build(
    manifests: Sequence[Path],
    *,
    stage: Path,
    version: str,
    output: Path,
    formats: Sequence[str] = FORMATS,
    arch: str = "amd64",
    table: Mapping[str, Mapping[str, str]] | None = None,
) -> list[str]:
    """Build every manifest in every format into output, and return the names of what was built."""
    table = load_table() if table is None else table
    output.mkdir(parents=True, exist_ok=True)
    environment = {**os.environ, "STAGE": str(stage.resolve()), "ARCH": arch}
    built: list[str] = []
    for manifest in manifests:
        for fmt in formats:
            print(f"[package] {package_name(manifest)} ({fmt})")
            source = manifest.read_text()
            scanned = scan_dependencies(packaged_files(source, stage.resolve()), fmt) if "@scan" in source else []
            text = resolve(source, fmt, table, str(manifest), scanned)
            with tempfile.TemporaryDirectory() as scratch:
                resolved = Path(scratch) / "nfpm.yaml"
                resolved.write_text(text)
                run(
                    ["nfpm", "package", "-f", resolved, "-p", fmt, "-t", output],
                    cwd=manifest.parent,
                    env={**environment, "VERSION": format_version(version, fmt)},
                )
        built.append(package_name(manifest))
    return built
