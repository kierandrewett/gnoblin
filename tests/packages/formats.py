"""Read the package files that pkgsmith writes: rpm, deb, Arch, apk and tar.

Each reader returns a Package: its metadata, its dependencies, the entries of its payload with their mode, owner and
link target, and a function that checks the checksums that the format embeds. The payload is extracted into a folder,
so the ELF tools can read the files. The rpm and deb readers start rpm, rpm2cpio, cpio and dpkg-deb. The Arch, apk and
tar readers need only the Python standard library and zstd.
"""

from __future__ import annotations

import dataclasses
import gzip
import hashlib
import io
import os
import re
import stat
import subprocess
import tarfile
import zlib
from pathlib import Path, PurePosixPath

# The format of a file, from its name. A source bundle is not a package.
SUFFIXES = (
    (".pkg.tar.zst", "archlinux"),
    (".rpm", "rpm"),
    (".deb", "deb"),
    (".apk", "apk"),
    (".tar.xz", "tar"),
)


def format_of(path: str | Path) -> str | None:
    name = Path(path).name
    if name.endswith("-source.tar.xz") or name.endswith(".src.rpm"):
        return None
    for suffix, fmt in SUFFIXES:
        if name.endswith(suffix):
            return fmt
    return None


@dataclasses.dataclass
class Dependency:
    name: str
    op: str = ""
    version: str = ""
    raw: str = ""
    alternatives: tuple[str, ...] = ()


@dataclasses.dataclass
class Entry:
    path: str  # absolute, as installed: /usr/bin/gnoblin
    kind: str  # file, dir or link
    mode: int  # permission bits, set-user-ID and set-group-ID included
    owner: str
    group: str
    target: str = ""  # link target


@dataclasses.dataclass
class Package:
    file: Path
    fmt: str
    name: str
    version: str  # without epoch and release
    release: str
    epoch: str
    arch: str
    license: str
    description: str
    depends: list[Dependency]
    recommends: list[Dependency]
    provides: list[str]
    conflicts: list[str]
    replaces: list[str]  # Replaces, Obsoletes
    entries: list[Entry]
    root: Path  # the extracted payload
    checksum_errors: list[str]
    checksum_count: int

    def files(self) -> list[Entry]:
        return [entry for entry in self.entries if entry.kind != "dir"]

    def local(self, entry_or_path: Entry | str) -> Path:
        path = entry_or_path.path if isinstance(entry_or_path, Entry) else entry_or_path
        return self.root / path.lstrip("/")


def _run(*command: str, stdin: bytes | None = None) -> bytes:
    return subprocess.run(command, input=stdin, capture_output=True, check=True).stdout


def _normal(path: str) -> str:
    """./usr/bin/x, usr/bin/x and /usr/bin/x/ are /usr/bin/x."""
    path = path.lstrip(".")
    return "/" + str(PurePosixPath("/" + path.strip("/"))).lstrip("/")


def _safe_target(root: Path, path: str) -> Path:
    clean = PurePosixPath(_normal(path))
    if ".." in clean.parts:
        raise ValueError(f"path with ..: {path}")
    return root / str(clean).lstrip("/")


def _extract_tar(tar: tarfile.TarFile, root: Path, skip=lambda member: False) -> list[Entry]:
    """Extract a tar stream by hand: no member may leave root, and links are kept as they are."""
    entries = []
    for member in tar:
        if skip(member):
            continue
        path = _normal(member.name)
        if path == "/":
            continue
        target = _safe_target(root, member.name)
        if member.isdir():
            kind = "dir"
            target.mkdir(parents=True, exist_ok=True)
        elif member.issym():
            kind = "link"
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.is_symlink() or target.exists():
                target.unlink()
            os.symlink(member.linkname, target)
        elif member.isreg():
            kind = "file"
            target.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(member) as source, open(target, "wb") as sink:
                sink.write(source.read())
            os.chmod(target, (member.mode & 0o777) | stat.S_IRUSR | stat.S_IWUSR)
        elif member.islnk():
            kind = "file"
            source = _safe_target(root, member.linkname)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
        else:
            raise ValueError(f"unexpected member type {member.type!r}: {member.name}")
        entries.append(
            Entry(
                path=path,
                kind=kind,
                mode=member.mode & 0o7777,
                owner=member.uname or str(member.uid),
                group=member.gname or str(member.gid),
                target=member.linkname if member.issym() else "",
            )
        )
    return entries


def _dep_list(text: str, separator: str = ",") -> list[Dependency]:
    """Debian syntax: name (op version) | other, ..."""
    result = []
    for item in text.split(separator):
        item = item.strip()
        if not item:
            continue
        alternatives = [part.strip() for part in item.split("|")]
        match = re.fullmatch(r"([^\s(]+)\s*(?:\(\s*([<>=]+)\s*([^)\s]+)\s*\))?", alternatives[0])
        if not match:
            raise ValueError(f"cannot read the dependency {item!r}")
        name = match.group(1).split(":", 1)[0]
        result.append(
            Dependency(
                name=name,
                op=match.group(2) or "",
                version=match.group(3) or "",
                raw=item,
                alternatives=tuple(re.split(r"[\s(]", alt, maxsplit=1)[0] for alt in alternatives),
            )
        )
    return result


def _op_dep(text: str) -> Dependency:
    """Arch and apk syntax: name, name>=version, name=version. rpm: name >= version."""
    match = re.fullmatch(r"\s*([^\s<>=~]+)\s*(?:([<>=~]+)\s*(\S+))?\s*", text)
    if not match:
        raise ValueError(f"cannot read the dependency {text!r}")
    return Dependency(name=match.group(1), op=match.group(2) or "", version=match.group(3) or "", raw=text.strip())


def read_rpm(path: Path, root: Path) -> Package:
    def query(fmt: str) -> str:
        return _run("rpm", "-qp", "--nosignature", "--nodigest", "--qf", fmt, str(path)).decode()

    def tags(option: str) -> list[str]:
        out = _run("rpm", "-qp", "--nosignature", "--nodigest", option, str(path)).decode()
        return [line for line in out.splitlines() if line.strip()]

    head = query("%{NAME}\\n%{VERSION}\\n%{RELEASE}\\n%{EPOCHNUM}\\n%{ARCH}\\n%{LICENSE}\\n%{SUMMARY}\\n")
    name, version, release, epoch, arch, licence, summary = head.splitlines()[:7]
    entries = []
    listing = query(
        "[%{FILEMODES}\\t%{FILEUSERNAME}\\t%{FILEGROUPNAME}\\t%{FILELINKTOS}\\t%{FILENAMES}\\t%{FILEFLAGS}\\n]"
    )
    ghosts = set()
    for line in listing.splitlines():
        if not line:
            continue
        mode, owner, group, link, filename, flags = line.split("\t")
        mode = int(mode)
        if int(flags) & (1 << 6):  # RPMFILE_GHOST: owned, not carried
            ghosts.add(filename)
            continue
        kind = "dir" if stat.S_ISDIR(mode) else "link" if stat.S_ISLNK(mode) else "file"
        entries.append(Entry(filename, kind, stat.S_IMODE(mode), owner, group, link if kind == "link" else ""))
    root.mkdir(parents=True, exist_ok=True)
    cpio = _run("rpm2cpio", str(path))
    subprocess.run(
        ["cpio", "-idm", "--quiet", "--no-absolute-filenames"], input=cpio, cwd=root, check=True, capture_output=True
    )
    errors = []
    result = subprocess.run(["rpm", "-K", "--nosignature", str(path)], capture_output=True, text=True)
    if result.returncode != 0 or "NOT OK" in result.stdout:
        errors.append(f"rpm -K --nosignature: {(result.stdout + result.stderr).strip()}")
    requires = [line for line in tags("--requires") if not line.startswith("rpmlib(")]
    return Package(
        file=path,
        fmt="rpm",
        name=name,
        version=version,
        release=release,
        epoch=epoch,
        arch=arch,
        license=licence,
        description=summary,
        depends=[_op_dep(line) if "(" not in line else Dependency(name=line, raw=line) for line in requires],
        recommends=[_op_dep(line) for line in tags("--recommends")],
        provides=tags("--provides"),
        conflicts=tags("--conflicts"),
        replaces=tags("--obsoletes"),
        entries=entries,
        root=root,
        checksum_errors=errors,
        checksum_count=sum(1 for entry in entries if entry.kind == "file") if not errors else 0,
    )


def _control_fields(text: str) -> dict[str, str]:
    fields, key = {}, None
    for line in text.splitlines():
        if line.startswith((" ", "\t")) and key:
            fields[key] += "\n" + line.strip()
        elif ":" in line:
            key, value = line.split(":", 1)
            fields[key.strip()] = value.strip()
    return fields


def read_deb(path: Path, root: Path) -> Package:
    control = _control_fields(_run("dpkg-deb", "-f", str(path)).decode())
    with tarfile.open(fileobj=io.BytesIO(_run("dpkg-deb", "--fsys-tarfile", str(path))), mode="r:") as tar:
        root.mkdir(parents=True, exist_ok=True)
        entries = _extract_tar(tar, root)
    with tarfile.open(fileobj=io.BytesIO(_run("dpkg-deb", "--ctrl-tarfile", str(path))), mode="r:") as tar:
        members = {_normal(member.name): member for member in tar if member.isreg()}
        md5sums = tar.extractfile(members["/md5sums"]).read().decode() if "/md5sums" in members else ""
    errors, count = [], 0
    listed = {}
    for line in md5sums.splitlines():
        digest, filename = line.split(None, 1)
        listed["/" + filename.strip()] = digest
    for entry in entries:
        if entry.kind != "file":
            continue
        digest = listed.get(entry.path)
        if digest is None:
            errors.append(f"{entry.path} has no md5sums entry")
            continue
        if hashlib.md5(root.joinpath(entry.path.lstrip("/")).read_bytes()).hexdigest() != digest:
            errors.append(f"{entry.path}: md5sums does not match")
            continue
        count += 1
    for filename in set(listed) - {entry.path for entry in entries}:
        errors.append(f"md5sums names {filename}, which the package does not hold")
    full = control.get("Version", "")
    epoch, _, rest = full.rpartition(":") if ":" in full else ("0", "", full)
    version, _, release = rest.rpartition("-") if "-" in rest else (rest, "", "")
    depends = _dep_list(control.get("Pre-Depends", "")) + _dep_list(control.get("Depends", ""))
    return Package(
        file=path,
        fmt="deb",
        name=control.get("Package", ""),
        version=version,
        release=release,
        epoch=epoch or "0",
        arch=control.get("Architecture", ""),
        license=_deb_licence(root, control.get("Package", "")),
        description=control.get("Description", "").split("\n", 1)[0],
        depends=depends,
        recommends=_dep_list(control.get("Recommends", "")),
        provides=[dep.raw for dep in _dep_list(control.get("Provides", ""))],
        conflicts=[dep.raw for dep in _dep_list(control.get("Conflicts", "")) + _dep_list(control.get("Breaks", ""))],
        replaces=[dep.raw for dep in _dep_list(control.get("Replaces", ""))],
        entries=entries,
        root=root,
        checksum_errors=errors,
        checksum_count=count,
    )


def _deb_licence(root: Path, name: str) -> str:
    """A deb has no licence field. pkgsmith writes it into the machine-readable copyright file."""
    copyright_file = root / f"usr/share/doc/{name}/copyright"
    if not copyright_file.is_file():
        return ""
    match = re.search(r"^License:\s*(\S.*)$", copyright_file.read_text(errors="replace"), re.MULTILINE)
    return match.group(1).strip() if match else ""


def _pkginfo(text: str) -> dict[str, list[str]]:
    fields: dict[str, list[str]] = {}
    for line in text.splitlines():
        if line.startswith("#") or " = " not in line:
            continue
        key, value = line.split(" = ", 1)
        fields.setdefault(key.strip(), []).append(value.strip())
    return fields


def _split_version(full: str) -> tuple[str, str, str]:
    epoch, _, rest = full.rpartition(":") if ":" in full else ("0", "", full)
    version, _, release = rest.rpartition("-")
    return epoch or "0", version, release


def read_arch(path: Path, root: Path) -> Package:
    data = _run("zstd", "-dcq", str(path))
    root.mkdir(parents=True, exist_ok=True)
    meta: dict[str, bytes] = {}

    def skip(member: tarfile.TarInfo) -> bool:
        name = member.name[2:] if member.name.startswith("./") else member.name
        if name.startswith("."):
            if member.isreg():
                meta[name[1:]] = tar.extractfile(member).read()
            return True
        return False

    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as tar:
        entries = _extract_tar(tar, root, skip)
    info = _pkginfo(meta.get("PKGINFO", b"").decode())
    errors, count = [], 0
    mtree = gzip.decompress(meta["MTREE"]).decode() if "MTREE" in meta else ""
    if not mtree:
        errors.append("no .MTREE")
    digests = {}
    for line in mtree.splitlines():
        if not line.startswith("./") or line.startswith("./."):
            continue
        name, *keywords = line.split()
        values = dict(word.split("=", 1) for word in keywords if "=" in word)
        if "sha256digest" in values:
            digests["/" + _unescape_mtree(name[2:])] = values["sha256digest"]
    for entry in entries:
        if entry.kind != "file":
            continue
        digest = digests.get(entry.path)
        if digest is None:
            errors.append(f"{entry.path} has no sha256digest in .MTREE")
        elif hashlib.sha256(root.joinpath(entry.path.lstrip("/")).read_bytes()).hexdigest() != digest:
            errors.append(f"{entry.path}: the sha256digest of .MTREE does not match")
        else:
            count += 1
    epoch, version, release = _split_version(info.get("pkgver", [""])[0])
    return Package(
        file=path,
        fmt="archlinux",
        name=info.get("pkgname", [""])[0],
        version=version,
        release=release,
        epoch=epoch,
        arch=info.get("arch", [""])[0],
        license=" ".join(info.get("license", [])),
        description=info.get("pkgdesc", [""])[0],
        depends=[_op_dep(value) for value in info.get("depend", [])],
        recommends=[_op_dep(value.split(":", 1)[0]) for value in info.get("optdepend", [])],
        provides=info.get("provides", []),
        conflicts=info.get("conflict", []),
        replaces=info.get("replaces", []),
        entries=entries,
        root=root,
        checksum_errors=errors,
        checksum_count=count,
    )


def _unescape_mtree(name: str) -> str:
    return re.sub(r"\\([0-7]{3})", lambda match: chr(int(match.group(1), 8)), name)


def _gzip_members(data: bytes) -> list[bytes]:
    """The raw gzip members of an apk: the signature (when signed), the control and the data segment."""
    members, rest = [], data
    while rest:
        decompressor = zlib.decompressobj(31)
        decompressor.decompress(rest)
        if not decompressor.eof:
            raise ValueError("truncated gzip member")
        used = len(rest) - len(decompressor.unused_data)
        members.append(rest[:used])
        rest = decompressor.unused_data
    return members


def read_apk(path: Path, root: Path) -> Package:
    members = _gzip_members(path.read_bytes())
    if len(members) < 2:
        raise ValueError(f"an apk has a control and a data segment, this one has {len(members)} gzip members")
    control_raw, data_raw = members[-2], members[-1]
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(control_raw)), mode="r:", ignore_zeros=True) as tar:
        control = {member.name: tar.extractfile(member).read() for member in tar if member.isreg()}
    info = _pkginfo(control.get(".PKGINFO", b"").decode())
    errors, count = [], 0
    datahash = info.get("datahash", [""])[0]
    if not datahash:
        errors.append("no datahash in .PKGINFO")
    elif hashlib.sha256(data_raw).hexdigest() != datahash:
        errors.append("the datahash of .PKGINFO does not match the data segment")
    root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(data_raw)), mode="r:", ignore_zeros=True) as tar:
        members_list = list(tar)
        for member in members_list:
            if member.isdir():
                continue
            checksum = member.pax_headers.get("APK-TOOLS.checksum.SHA1")
            if not checksum:
                kind = "symbolic link" if member.issym() else "file"
                errors.append(f"/{member.name} ({kind}) has no APK-TOOLS.checksum.SHA1")
                continue
            if member.issym():
                actual = hashlib.sha1(member.linkname.encode()).hexdigest()
            elif member.isreg():
                actual = hashlib.sha1(tar.extractfile(member).read()).hexdigest()
            else:
                continue
            if actual != checksum:
                errors.append(f"/{member.name}: APK-TOOLS.checksum.SHA1 does not match")
            else:
                count += 1
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(data_raw)), mode="r:", ignore_zeros=True) as tar:
        entries = _extract_tar(tar, root)
    full = info.get("pkgver", [""])[0]
    version, _, release = full.rpartition("-")
    return Package(
        file=path,
        fmt="apk",
        name=info.get("pkgname", [""])[0],
        version=version,
        release=release,
        epoch="0",
        arch=info.get("arch", [""])[0],
        license=" ".join(info.get("license", [])),
        description=info.get("pkgdesc", [""])[0],
        depends=[_op_dep(value) for value in info.get("depend", [])],
        recommends=[],
        provides=info.get("provides", []),
        conflicts=[value[1:] for value in info.get("depend", []) if value.startswith("!")],
        replaces=info.get("replaces", []),
        entries=entries,
        root=root,
        checksum_errors=errors,
        checksum_count=count,
    )


TAR_NAME = re.compile(r"^(?P<name>.+?)-(?P<version>\d[^-]*(?:-[a-z][^-]*)*)-(?P<arch>[a-z0-9_]+)\.tar\.xz$")


def read_tar(path: Path, root: Path) -> Package:
    """A tar package holds the files only. Its name, version and architecture are in the file name."""
    match = TAR_NAME.match(path.name)
    if not match:
        raise ValueError(f"cannot read the name, version and architecture of {path.name}")
    root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path, mode="r:xz") as tar:
        entries = _extract_tar(tar, root)
    return Package(
        file=path,
        fmt="tar",
        name=match.group("name"),
        version=match.group("version"),
        release="",
        epoch="0",
        arch=match.group("arch"),
        license="",
        description="",
        depends=[],
        recommends=[],
        provides=[],
        conflicts=[],
        replaces=[],
        entries=entries,
        root=root,
        checksum_errors=[],
        checksum_count=0,
    )


READERS = {"rpm": read_rpm, "deb": read_deb, "archlinux": read_arch, "apk": read_apk, "tar": read_tar}


def read(path: Path, root: Path) -> Package:
    fmt = format_of(path)
    if fmt is None:
        raise ValueError(f"not a package file: {path.name}")
    return READERS[fmt](path, root)
