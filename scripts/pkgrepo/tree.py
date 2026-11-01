"""Assemble a repository tree: merge new packages with the published ones, apply retention, write and sign metadata.

Layout, for one project and channel (design/package-repositories.md has the reasoning):

    <root>/gpg.key
    <root>/<project>.rsa.pub                                  the key apk checks the index with, when there are apk packages
    <root>/<project>/<channel>/manifest.json
    <root>/<project>/<channel>/fedora/<release>/<arch>/    RPMs and repodata/
    <root>/<project>/<channel>/opensuse/<release>/<arch>/  RPMs and repodata/
    <root>/<project>/<channel>/deb/                         dists/<suite>/main/binary-<arch>/ and pool/<suite>/
    <root>/<project>/<channel>/arch/<arch>/                 packages, signatures and <name>.db
    <root>/<project>/<channel>/alpine/<branch>/<arch>/      .apk files and the signed APKINDEX.tar.gz
"""

from __future__ import annotations

import json
import shutil
import sys
import urllib.error
import urllib.parse
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

from . import SCHEMA, alpine, download, formats, index
from .packages import Package, scan, sha256_of
from .signing import ApkSigner, Signer
from .versions import CHANNELS, NIGHTLY, STABLE, comparator

DEBIAN_ARCH = {"x86_64": "amd64", "aarch64": "arm64"}


class TreeError(RuntimeError):
    pass


@dataclass
class Entry:
    package: Package
    source: Path
    new: bool
    signature: Path | None = None


@dataclass
class Settings:
    project: str
    channel: str
    input_dir: Path
    output: Path
    existing: Path | None = None
    keep_nightlies: int = 14
    replace_stable: bool = False
    source_commit: str = ""
    source_ref: str = ""
    base_url: str = "https://repo.gnoblin.org"
    default_arch: str = "x86_64"
    now: datetime = field(default_factory=lambda: datetime.now(timezone.utc))


def channel_root(root: Path, project: str, channel: str) -> Path:
    return root / project / channel


def pacman_repo_name(project: str, channel: str) -> str:
    """The pacman.conf section name of a channel; the database file carries it."""
    return project if channel == STABLE else f"{project}-{channel}"


def apk_key_name(project: str) -> str:
    """The file name of the Alpine public key. apk looks for the key by the name in the index signature."""
    return f"{project}.rsa.pub"


def read_manifest(path: Path) -> dict:
    data = json.loads(path.read_text())
    if data.get("schema") != SCHEMA:
        raise TreeError(f"{path}: unsupported manifest schema {data.get('schema')!r}")
    return data


def load_existing(existing: Path | None, project: str, channel: str) -> list[Entry]:
    """The packages of a pulled copy of the published channel (see `pkgrepo pull`)."""
    if existing is None:
        return []
    root = channel_root(existing, project, channel)
    manifest = root / "manifest.json"
    if not manifest.exists():
        return []
    entries = []
    for item in read_manifest(manifest)["packages"]:
        fields = {key: item[key] for key in Package.__dataclass_fields__}
        package = Package(**fields)
        source = root / item["path"]
        if not source.exists():
            raise TreeError(f"{manifest} lists {item['path']}, which is missing from {root}; run pkgrepo pull again")
        if sha256_of(source) != package.sha256:
            raise TreeError(f"{source} does not match the sha256 in {manifest}")
        signature = root / item["signature"] if item.get("signature") else None
        entries.append(Entry(package, source, new=False, signature=signature))
    return entries


def merge(existing: list[Entry], fresh: list[Entry], channel: str, replace_stable: bool) -> list[Entry]:
    merged = {(e.package.group, e.package.version): e for e in existing}
    published = set(merged)
    for entry in fresh:
        key = (entry.package.group, entry.package.version)
        # An architecture-independent Alpine package is built once per architecture under one file name, so the same
        # package can come twice in one batch. Only a package that was published before counts as a replacement.
        if key in published and channel == STABLE and not replace_stable:
            package = entry.package
            raise TreeError(
                f"{package.name} {package.version} ({package.family} {package.release or package.arch}) is already published in "
                "the stable channel. Stable files are never replaced: raise the release number, or pass --replace-stable "
                "when this is a deliberate repair"
            )
        merged[key] = entry
    return list(merged.values())


def apply_retention(entries: list[Entry], channel: str, keep: int) -> tuple[list[Entry], list[Entry]]:
    """Keep every stable package. In the nightly channel keep the newest `keep` versions of each package."""
    if channel != NIGHTLY:
        return entries, []
    if keep < 1:
        raise TreeError("--keep-nightlies must be at least 1")
    groups: dict[tuple, list[Entry]] = {}
    for entry in entries:
        groups.setdefault(entry.package.group, []).append(entry)
    kept: list[Entry] = []
    dropped: list[Entry] = []
    from functools import cmp_to_key

    for group, members in groups.items():
        compare = comparator(group[-1])
        members.sort(key=cmp_to_key(lambda a, b: compare(a.package.version, b.package.version)), reverse=True)
        kept.extend(members[:keep])
        dropped.extend(members[keep:])
    return kept, dropped


def default_for(family: str, default: str) -> str:
    return DEBIAN_ARCH.get(default, default) if family == "deb" else default


def _arch_dirs(entries: list[Entry], default: str) -> dict[tuple[str, str], list[str]]:
    """For each (family, release), the architecture directories it needs, sorted.

    Architecture-independent packages (noarch, any) go into every directory of their release. A release that has only
    such packages gets the default architecture. Debian has one pool per suite, so it needs no directories.
    """
    result: dict[tuple[str, str], set[str]] = {}
    for entry in entries:
        package = entry.package
        archs = result.setdefault((package.family, package.release), set())
        if not package.independent and package.format != "deb":
            archs.add(package.arch)
    return {key: sorted(archs or {default}) for key, archs in result.items()}


def canonical_path(package: Package, arch_dirs: dict[tuple[str, str], list[str]]) -> str:
    """The path of the package file within the channel; arch-independent packages use the first architecture."""
    if package.format == "deb":
        return f"deb/pool/{package.release}/{package.filename}"
    arch = package.arch if not package.independent else arch_dirs[(package.family, package.release)][0]
    if package.format == "arch":
        return f"arch/{arch}/{package.filename}"
    return f"{package.family}/{package.release}/{arch}/{package.filename}"


def build(settings: Settings, signer: Signer, apk_signer: ApkSigner | None = None) -> dict:
    if settings.channel not in CHANNELS:
        raise TreeError(f"the channel must be one of {', '.join(CHANNELS)}")
    scanned = [Entry(package, path, new=True) for package, path in scan(settings.input_dir)]
    if not scanned:
        raise TreeError(f"no packages found under {settings.input_dir}")
    existing = load_existing(settings.existing, settings.project, settings.channel)
    entries = merge(existing, scanned, settings.channel, settings.replace_stable)
    entries, dropped = apply_retention(entries, settings.channel, settings.keep_nightlies)
    for entry in dropped:
        print(f"retention: dropping {entry.package.name} {entry.package.version}", flush=True)

    if apk_signer is None and any(e.package.format == "apk" for e in entries):
        raise TreeError(
            "the channel has Alpine packages and no key to sign their index with: set REPO_APK_PRIVATE_KEY or pass --apk-key"
        )
    root = channel_root(settings.output, settings.project, settings.channel)
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    arch_dirs = _arch_dirs(entries, settings.default_arch)

    # 1. Lay the package files out.
    placed: dict[Path, list[Entry]] = {}
    for entry in entries:
        package = entry.package
        paths = _destinations(package, arch_dirs)
        for relative in paths:
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(entry.source, destination)
            if entry.signature is not None:
                shutil.copyfile(entry.signature, destination.with_name(destination.name + ".sig"))
            placed.setdefault(destination.parent, []).append(entry)

    # 2. Per directory: sign, write and sign the metadata.
    for directory in sorted(placed):
        members = placed[directory]
        family = members[0].package.family
        if family in ("fedora", "opensuse"):
            formats.build_rpm(directory, signer, {e.package.filename for e in members if e.new})
        elif family == "arch":
            formats.build_pacman(
                directory,
                pacman_repo_name(settings.project, settings.channel),
                signer,
                sorted({e.package.filename for e in members}),
            )
        elif family == "alpine":
            alpine.build_index(
                directory,
                apk_signer,
                sorted({e.package.filename for e in members}),
                f"{settings.project.capitalize()} {settings.channel}",
            )
    for suite, members in _suites(entries).items():
        architectures = sorted(
            {e.package.arch for e in members if not e.package.independent}
            or {default_for("deb", settings.default_arch)}
        )
        formats.build_apt_suite(
            root / "deb",
            suite,
            architectures,
            origin=settings.project.capitalize(),
            label=f"{settings.project.capitalize()} {settings.channel}",
            description=f"{settings.project.capitalize()} {settings.channel} packages for {suite}",
            signer=signer,
        )

    # 3. Record what is in the tree. The checksums are taken after signing: rpmsign rewrites the file.
    records = []
    for entry in sorted(entries, key=lambda e: canonical_path(e.package, arch_dirs)):
        relative = canonical_path(entry.package, arch_dirs)
        file = root / relative
        record = entry.package.as_dict()
        record.update(
            path=relative,
            size=file.stat().st_size,
            sha256=sha256_of(file),
            signature=relative + ".sig" if (root / (relative + ".sig")).exists() else None,
        )
        records.append(record)
    manifest = {
        "schema": SCHEMA,
        "project": settings.project,
        "channel": settings.channel,
        "generated": settings.now.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "source": {"commit": settings.source_commit, "ref": settings.source_ref},
        "key": {"fingerprint": signer.fingerprint, "url": f"{settings.base_url}/gpg.key"},
        "packages": records,
    }
    if apk_signer is not None and any(e.package.format == "apk" for e in entries):
        manifest["apk_key"] = {"name": apk_signer.name, "url": f"{settings.base_url}/{apk_signer.name}"}
        apk_signer.write_public(settings.output / apk_signer.name)
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")

    # 4. Pages and client configuration files, then the files at the root of the whole site.
    index.write_channel(settings, root, manifest, arch_dirs)
    signer.export_public(settings.output / "gpg.key")
    index.write_root(settings, manifest["key"]["fingerprint"], manifest.get("apk_key", {}).get("name"))
    return manifest


def _destinations(package: Package, arch_dirs: dict[tuple[str, str], list[str]]) -> list[str]:
    if package.format == "deb":
        return [f"deb/pool/{package.release}/{package.filename}"]
    if package.independent:
        directories = arch_dirs[(package.family, package.release)]
    else:
        directories = [package.arch]
    if package.format == "arch":
        return [f"arch/{arch}/{package.filename}" for arch in directories]
    return [f"{package.family}/{package.release}/{arch}/{package.filename}" for arch in directories]


def _suites(entries: list[Entry]) -> dict[str, list[Entry]]:
    suites: dict[str, list[Entry]] = {}
    for entry in entries:
        if entry.package.format == "deb":
            suites.setdefault(entry.package.release, []).append(entry)
    return suites


# --- pull ------------------------------------------------------------------------------------------------------------


def _fetch(url: str, destination: Path | None = None) -> bytes | None:
    try:
        with download.open_url(url) as response:
            data = response.read()
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    if destination is not None:
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    return data


def pull(
    url: str, project: str, channel: str, output: Path, allow_missing: bool, allow_unreachable: bool = False
) -> int:
    """Download the manifest and the package files of a published channel, so a build can merge with them.

    Only the manifest and the files it lists are fetched. Metadata is rebuilt on every run, so it is not.
    Returns the number of packages pulled; a channel that is not published yet is an error unless allow_missing.
    """
    base = f"{url.rstrip('/')}/{project}/{channel}"
    root = channel_root(output, project, channel)
    try:
        manifest_bytes = _fetch(f"{base}/manifest.json")
    except urllib.error.URLError as error:
        if not allow_unreachable:
            raise TreeError(f"cannot reach {base}/manifest.json: {error.reason}") from error
        print(
            f"pkgrepo: cannot reach {base} ({error.reason}); building without the published packages", file=sys.stderr
        )
        return 0
    if manifest_bytes is None:
        if allow_missing:
            return 0
        raise TreeError(f"{base}/manifest.json does not exist. Pass --allow-missing for the first publication")
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema") != SCHEMA:
        raise TreeError(f"{base}/manifest.json: unsupported schema {manifest.get('schema')!r}")
    root.mkdir(parents=True, exist_ok=True)
    (root / "manifest.json").write_bytes(manifest_bytes)
    for item in manifest["packages"]:
        for relative in filter(None, (item["path"], item.get("signature"))):
            destination = root / relative
            if _fetch(f"{base}/{urllib.parse.quote(relative)}", destination) is None:
                raise TreeError(f"{base}/{relative} is listed in the manifest but does not exist")
        if sha256_of(root / item["path"]) != item["sha256"]:
            raise TreeError(f"{item['path']} does not match the sha256 in the published manifest")
    return len(manifest["packages"])
