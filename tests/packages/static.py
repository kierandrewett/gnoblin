#!/usr/bin/env python3
"""Layer 1 of the package suite: check the package files of one target without installing them.

usage: tests/packages/static.py --target FILE --packages DIR --report DIR [--version SEMVER] [--release N]
                                [--rebuilt DIR] [--work DIR]

FILE is one entry of the matrix of scripts/supported-targets.py (JSON). DIR is the artifact of the target. --version
is the version the packages were written with (default: src/gnoblin-version.json). --rebuilt is a folder that holds
the same packages written a second time from the same prefix; without it the reproducibility case is skipped.
design/package-testing.md lists the failure modes S1 to S13 that the cases detect.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))

import elf  # noqa: E402
import formats  # noqa: E402
import targets  # noqa: E402
from report import Layer  # noqa: E402

EXPECT = targets.EXPECTATIONS
PRIVATE = "/usr/lib/gnoblin"


def load_isolation():
    """The path and relation rules of scripts/check-rpm-isolation.py, which this layer applies to every format."""
    spec = importlib.util.spec_from_file_location("check_rpm_isolation", ROOT / "scripts/check-rpm-isolation.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ISOLATION = load_isolation()
# The public entries and the package that holds each, as the install guide and the install layer expect them.
PUBLIC_OWNER = {
    "/usr/share/xdg-desktop-portal/portals/gnoblin.portal": "xdg-desktop-portal-gnoblin",
    "/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service": "xdg-desktop-portal-gnoblin",
    "/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service": "xdg-desktop-portal-gnoblin",
}
OLD_NAMES = {item.split()[0] for item in ISOLATION.ALLOWED_OBSOLETES}


def gnoblin_version() -> str:
    return json.loads((ROOT / "src/gnoblin-version.json").read_text())["version"]


def portal_version() -> str:
    version = subprocess.run(
        [sys.executable, str(ROOT / "scripts/gnome-versions.py"), "get", "xdg-desktop-portal-gnome", "version"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    while version.count(".") < 2:
        version += ".0"
    return version


def numeric_release(version: str) -> str:
    """The release number of the portal backend: 0.1.13 is 1013 (pkgsmith/common.star numeric_release)."""
    major, minor, patch = (int(part) for part in version.split("-", 1)[0].split(".")[:3])
    return str(major * 1000000 + minor * 1000 + patch)


def _digits_without_commit(text: str) -> str:
    return re.sub(r"\D", "", re.sub(r"(?<![0-9a-z])g[0-9a-f]{7,40}(?![0-9a-z])", "", text))


def version_matches(spelled: str, semver: str) -> bool:
    """A package spells a nightly in its own way (0.1.13~nightly.20261101.48.gabc1234, 0.1.13nightly20261101.48,
    0.1.13_pre2026110148). It matches when it starts with the release numbers and holds the same nightly numbers."""
    core, _, pre = semver.partition("-")
    if not spelled.startswith(core):
        return False
    rest = spelled[len(core) :]
    if not pre:
        return rest == ""
    return rest != "" and not rest[0].isdigit() and _digits_without_commit(rest) == _digits_without_commit(pre)


class PackageSet:
    """The packages of one format of the target, read and extracted."""

    def __init__(self, fmt: str, packages: list[formats.Package]):
        self.fmt = fmt
        self.packages = packages
        self.by_name = {package.name: package for package in packages}
        self.paths: set[str] = set()
        for package in packages:
            for entry in package.entries:
                self.paths.add(entry.path)
                for parent in PurePosixPath(entry.path).parents:
                    self.paths.add(str(parent))
        self.elves: dict[str, dict[str, elf.ElfInfo]] = {}
        for package in packages:
            found = {}
            for entry in package.files():
                if entry.kind == "file" and elf.is_elf(package.local(entry)):
                    found[entry.path] = elf.read(package.local(entry))
            self.elves[package.name] = found
        self.internal: set[str] = set()
        for package in packages:
            for path, info in self.elves[package.name].items():
                if info.soname:
                    self.internal.add(info.soname)
            for entry in package.entries:
                if entry.path.startswith(PRIVATE + "/lib/") and ".so" in PurePosixPath(entry.path).name:
                    self.internal.add(PurePosixPath(entry.path).name)

    def link_target(self, entry: formats.Entry) -> str | None:
        """Where a link points, as an absolute path, or None when it leaves the file system by ..."""
        base = PurePosixPath(entry.path).parent
        joined = PurePosixPath(entry.target) if entry.target.startswith("/") else base / entry.target
        parts: list[str] = []
        for part in joined.parts[1:]:
            if part == "..":
                if not parts:
                    return None
                parts.pop()
            elif part not in (".", ""):
                parts.append(part)
        return "/" + "/".join(parts)


def check_set(layer: Layer, fmt: str, files: list[Path]):
    names = {}
    for path in files:
        names.setdefault(path.name, path)
    errors = []
    found: dict[str, list[str]] = {}
    for path in files:
        found.setdefault(package_name_from_file(path), []).append(path.name)
    for name in EXPECT["packages"]:
        if name not in found:
            errors.append(f"no {name} package")
        elif len(found[name]) > 1:
            errors.append(f"{name} is there {len(found[name])} times: {', '.join(found[name])}")
    for name in sorted(set(found) - set(EXPECT["packages"])):
        errors.append(f"unknown package {name}: {', '.join(found[name])}")
    layer.check("S1", "package set complete", fmt, errors)


def package_name_from_file(path: Path) -> str:
    """The package name from the file name: the reader confirms it from the metadata (S2)."""
    name = path.name
    for candidate in sorted(EXPECT["packages"], key=len, reverse=True):
        if re.match(rf"^{re.escape(candidate)}[-_]\d", name):
            return candidate
    return re.split(r"[-_]\d", name, maxsplit=1)[0]


def deb_copyright_errors(pkg, licence: str) -> list[str]:
    """A deb has no licence field. Debian policy wants /usr/share/doc/<package>/copyright, and the machine-readable
    format (DEP-5) names the licence of the files: the "Files: *" paragraph must name the licence of the package, and
    every licence that a paragraph names needs its own text paragraph."""
    path = f"/usr/share/doc/{pkg.name}/copyright"
    if path not in {entry.path for entry in pkg.files()}:
        return [f"no {path}"]
    text = pkg.local(path).read_text(errors="replace")
    paragraphs = [block for block in re.split(r"\n\s*\n", text.strip()) if block.strip()]
    format_line = "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    if not paragraphs or not paragraphs[0].startswith(format_line):
        return [f"{path} is not in the machine-readable format (DEP-5)"]
    errors, named, texts = [], set(), set()
    for block in paragraphs[1:]:
        files = re.search(r"^Files:\s*(.+)$", block, re.MULTILINE)
        licence_line = re.search(r"^License:\s*(\S+)\s*$", block, re.MULTILINE)
        if files and licence_line:
            named.add(licence_line.group(1))
            if files.group(1).strip() == "*" and licence_line.group(1) != licence:
                errors.append(f"{path}: Files: * has the licence {licence_line.group(1)}, expected {licence}")
        elif licence_line and not files:
            texts.add(licence_line.group(1))
            if re.match(r"L?GPL-", licence_line.group(1)) and "/usr/share/common-licenses/" not in block:
                errors.append(f"{path}: the {licence_line.group(1)} text does not refer to /usr/share/common-licenses")
    if not re.search(r"^Files:\s*\*\s*$", text, re.MULTILINE):
        errors.append(f"{path} has no Files: * paragraph")
    for missing in sorted(named - texts):
        errors.append(f"{path} names {missing} and gives no text for it")
    return errors


def check_metadata(layer, target, pkg, pset, versions, release_number):
    errors = []
    expected = EXPECT["packages"].get(pkg.name)
    subject = pkg.file.name
    if expected is None:
        layer.check("S2", "metadata", subject, [f"unknown package name {pkg.name!r}"])
        return
    if package_name_from_file(pkg.file) != pkg.name:
        errors.append(f"the file name says {package_name_from_file(pkg.file)}, the metadata says {pkg.name}")
    version = versions[expected["version"]]
    if expected["version"] == "portal":
        if pkg.version != version:
            errors.append(f"version {pkg.version}, expected {version}")
        number = numeric_release(versions["gnoblin"])
    else:
        if not version_matches(pkg.version, version):
            errors.append(f"version {pkg.version} does not spell {version}")
        number = release_number
    slug = target["slug"]
    if pkg.fmt == "rpm":
        want = f"{number}.{slug}"
    elif pkg.fmt == "deb":
        want = f"{number}~{slug}" if targets.own_lane(target) else number
    elif pkg.fmt == "apk":
        want = f"r{number}"
    elif pkg.fmt == "archlinux":
        want = number
    else:
        want = ""
    if pkg.release != want:
        errors.append(f"release {pkg.release!r}, expected {want!r}")
    has_elf = bool(pset.elves[pkg.name])
    arch_names = EXPECT["architectures"][target["arch"]]
    if expected["compiled"]:
        if pkg.arch != arch_names[pkg.fmt]:
            errors.append(f"architecture {pkg.arch!r}, expected {arch_names[pkg.fmt]!r}")
        if not has_elf:
            errors.append("holds no ELF file, but it is the package of compiled code")
    else:
        if pkg.arch != EXPECT["independent"][pkg.fmt]:
            errors.append(f"architecture {pkg.arch!r}, expected {EXPECT['independent'][pkg.fmt]!r}")
        if has_elf:
            errors.append(f"is architecture-independent but holds ELF files: {', '.join(sorted(pset.elves[pkg.name]))}")
    if pkg.fmt in ("rpm", "archlinux", "apk"):
        if pkg.license != expected["licence"]:
            errors.append(f"licence {pkg.license!r}, expected {expected['licence']!r}")
    elif pkg.fmt == "deb":
        errors += deb_copyright_errors(pkg, expected["licence"])
    if pkg.fmt != "tar" and not pkg.description.strip():
        errors.append("no description")
    layer.check("S2", "metadata", subject, errors)


def check_dependencies(layer, target, pkg, pset):
    errors = []
    for dep in pkg.depends:
        name = dep.name
        if name.startswith("pkgconfig(") or re.search(r"-(devel|dev)$", name):
            errors.append(f"asks for a development package: {dep.raw}")
    # The compositor draws the frames of X11 apps itself (no GTK frames client), so only the portal backend needs GTK.
    if pkg.name == "gnoblin":
        for dep in pkg.depends + pkg.recommends:
            if re.search(r"(^|[:(_-])(lib)?(gtk|adwaita)", dep.name.lower()):
                errors.append(f"gnoblin asks for GTK or libadwaita: {dep.raw}. Only the portal backend may.")
    for capability in pkg.provides:
        if re.match(ISOLATION_PROVIDES, capability):
            errors.append(f"provides a system capability: {capability}")
    relations = pkg.conflicts + pkg.replaces
    for relation in relations:
        other = re.split(r"[\s(<>=]", relation, maxsplit=1)[0]
        if pkg.name == "gnoblin" and other in OLD_NAMES:
            continue
        errors.append(f"declares a conflict with or replaces {relation}")
    layer.check("S3", "dependencies", pkg.file.name, errors)


ISOLATION_PROVIDES = (
    r"(?:mutter|gnome-shell|libmutter|libshell-|libst-|pkgconfig\(|desktop-notification-daemon|"
    r"PolicyKit-authentication-agent)"
)


def check_paths(layer, pkg):
    errors = []
    licence_dirs = (f"/usr/share/licenses/{pkg.name}", f"/usr/share/doc/{pkg.name}")
    config = ISOLATION.CONFIG_FILES.get(pkg.name, ())
    for entry in pkg.entries:
        path = entry.path
        if entry.kind == "dir":
            continue
        if ".." in PurePosixPath(path).parts:
            errors.append(f"path with ..: {path}")
            continue
        if path in ISOLATION.PUBLIC_FILES:
            owner = PUBLIC_OWNER.get(path, "gnoblin")
            if pkg.name != owner:
                errors.append(f"{path} belongs to {owner}")
            continue
        if pkg.name == "gnoblin" and ISOLATION.MAN_PAGE.match(path):
            continue
        if path.startswith(PRIVATE + "/") or path.startswith(tuple(prefix + "/" for prefix in licence_dirs)):
            continue
        if path.startswith(config):
            continue
        if pkg.fmt == "rpm" and path.startswith("/usr/lib/.build-id/"):
            continue
        errors.append(f"installs outside Gnoblin's paths: {path}")
    layer.check("S4", "file paths and stock GNOME files", pkg.file.name, errors)


def check_modes(layer, pkg, pset):
    errors = []
    for entry in pkg.entries:
        if entry.kind == "link":
            continue
        if entry.owner not in ("root", "0") or entry.group not in ("root", "0"):
            errors.append(f"{entry.path} is owned by {entry.owner}:{entry.group}")
        if entry.mode & (stat.S_ISUID | stat.S_ISGID):
            errors.append(f"{entry.path} is set-user-ID or set-group-ID ({entry.mode:o})")
        if entry.mode & 0o022:
            errors.append(f"{entry.path} is writable by group or others ({entry.mode:o})")
        if entry.kind == "file" and entry.mode & 0o444 != 0o444:
            errors.append(f"{entry.path} is not readable by everyone ({entry.mode:o})")
        if entry.kind == "dir" and entry.mode & 0o555 != 0o555:
            errors.append(f"{entry.path} is a folder that not everyone can enter ({entry.mode:o})")
        info = pset.elves[pkg.name].get(entry.path)
        if info and info.program and entry.mode & 0o111 != 0o111:
            errors.append(f"{entry.path} is a program without the execute bits ({entry.mode:o})")
    layer.check("S5", "owners and modes", pkg.file.name, errors)


def check_links(layer, pkg, pset):
    errors = []
    links = {entry.path: entry for p in pset.packages for entry in p.entries if entry.kind == "link"}
    for entry in pkg.entries:
        if entry.kind != "link":
            continue
        current, seen = entry, set()
        while True:
            resolved = pset.link_target(current)
            if resolved is None:
                errors.append(f"{entry.path} -> {entry.target} leaves the file system")
                break
            if resolved not in pset.paths:
                errors.append(f"{entry.path} -> {entry.target}: {resolved} is not in the package set")
                break
            if resolved in links and resolved not in seen:
                seen.add(resolved)
                current = links[resolved]
                continue
            if resolved in seen:
                errors.append(f"{entry.path} -> {entry.target} is a loop")
            break
    layer.check("S6", "symbolic links resolve inside the package set", pkg.file.name, errors)


def check_build_paths(layer, pkg):
    patterns = [re.compile(pattern.encode()) for pattern in EXPECT["build_paths"]]
    allowed = [re.compile(item["pattern"].encode()) for item in EXPECT["build_path_allow"]]
    errors = []
    for entry in pkg.files():
        if entry.kind != "file":
            continue
        data = pkg.local(entry).read_bytes()
        for pattern in patterns:
            for match in pattern.finditer(data):
                start = max(0, match.start() - 40)
                context = data[start : match.end() + 40]
                if any(allow.search(context) for allow in allowed):
                    continue
                text = re.sub(rb"[^\x20-\x7e]", b".", context).decode()
                errors.append(f"{entry.path}: {text}")
                break
    layer.check("S7", "no build machine paths", pkg.file.name, errors)


def check_checksums(layer, pkg):
    if pkg.fmt == "tar":
        layer.check(
            "S12",
            "embedded checksums",
            pkg.file.name,
            [],
            skip="the tar format embeds no checksums; SHA256SUMS of the release covers the file",
        )
        return
    errors = list(pkg.checksum_errors)
    if not errors and pkg.checksum_count == 0 and pkg.files():
        errors.append("no checksum was checked")
    layer.check("S12", "embedded checksums", pkg.file.name, errors)


def check_abi(layer, pkg, pset):
    elves = pset.elves[pkg.name]
    subject = pkg.file.name
    if not elves:
        return
    if pkg.fmt == "tar":
        layer.check(
            "S13",
            "ABI and dependencies",
            subject,
            [],
            skip="a tar package has no dependencies; the install layer resolves it (I15)",
        )
        return
    needed: set[str] = set()
    nodes: dict[str, set[str]] = {}
    for info in elves.values():
        needed.update(info.needed)
        for library, names in info.versions.items():
            nodes.setdefault(library, set()).update(names)
    needed = {name for name in needed if name not in pset.internal and not name.startswith(("ld-linux", "ld-musl"))}
    errors = []
    if pkg.fmt in ("rpm", "apk"):
        if pkg.fmt == "rpm":
            declared = {
                match.group(1)
                for dep in pkg.depends
                if (match := re.match(r"^(\S+?\.so(?:\.[\w.]+)?)\(.*\)(?:\(64bit\))?$", dep.name))
            }
        else:
            declared = {dep.name[3:] for dep in pkg.depends if dep.name.startswith("so:")}
        for name in sorted(needed - declared):
            errors.append(f"an ELF file needs {name}, which the package does not ask for")
        for name in sorted(declared - needed):
            errors.append(f"asks for {name}, which no ELF file of the package needs")
    for prefix, floor in EXPECT["symbol_floors"].items():
        if prefix == "comment":
            continue
        wanted = [elf.version_number(node, prefix) for node in nodes.get(floor["soname"], set())]
        wanted = [number for number in wanted if number]
        if not wanted:
            continue
        highest = ".".join(str(part) for part in max(wanted))
        if pkg.fmt == "apk":
            continue  # apk names the library without a version: the install layer resolves the symbols (I4)
        if pkg.fmt == "rpm":
            asked = [
                match.group(1)
                for dep in pkg.depends
                if (
                    match := re.fullmatch(
                        re.escape(floor["soname"]) + rf"\({prefix}_([\d.]+)\)(?:\(64bit\))?", dep.name
                    )
                )
            ]
        else:
            asked = [dep.version for dep in pkg.depends if dep.name == floor[pkg.fmt] and dep.op in (">=", ">")]
        if not asked:
            errors.append(f"needs {prefix}_{highest} from {floor['soname']}, and asks for no version of it")
        elif max(elf.compare_versions(version, highest) for version in asked) < 0:
            errors.append(f"needs {prefix}_{highest} from {floor['soname']}, and asks only for {', '.join(asked)}")
    layer.check("S13", "ABI and dependencies", subject, errors)


def desktop_message_allowed(path: str, line: str) -> bool:
    """A message of desktop-file-validate that expectations.json accepts for this kind of entry, with its reason."""
    return any(item["path"] in path and re.search(item["pattern"], line) for item in EXPECT["desktop_validate_allow"])


def check_desktop_files(layer, pset):
    tool = shutil.which("desktop-file-validate")
    errors = []
    found = 0
    for pkg in pset.packages:
        for entry in pkg.files():
            if entry.kind == "file" and entry.path.endswith(".desktop"):
                found += 1
                if not tool:
                    continue
                result = subprocess.run([tool, str(pkg.local(entry))], capture_output=True, text=True)
                lines = [
                    line.replace(str(pkg.local(entry)), entry.path)
                    for line in (result.stdout + result.stderr).splitlines()
                    if line.strip() and not desktop_message_allowed(entry.path, line)
                ]
                if lines:
                    errors.append(f"{entry.path}: {'; '.join(lines)}")
    if not tool:
        errors.append("desktop-file-validate is not installed on this machine")
    if not found:
        errors.append("the package set holds no .desktop file, so the session entry is missing")
    layer.check("S8", "desktop entries validate", pset.fmt, errors)


def check_schemas(layer, pset, work: Path):
    tool = shutil.which("glib-compile-schemas")
    directory = work / f"schemas-{pset.fmt}"
    directory.mkdir(parents=True, exist_ok=True)
    count = 0
    for pkg in pset.packages:
        for entry in pkg.files():
            if (
                entry.kind == "file"
                and entry.path.startswith(PRIVATE + "/share/glib-2.0/schemas/")
                and entry.path.endswith((".gschema.xml", ".gschema.override", ".enums.xml"))
            ):
                shutil.copy(pkg.local(entry), directory / PurePosixPath(entry.path).name)
                count += 1
    errors = []
    if not tool:
        errors.append("glib-compile-schemas is not installed on this machine")
    elif not count:
        errors.append("the package set holds no GSettings schema")
    else:
        result = subprocess.run([tool, "--strict", "--dry-run", str(directory)], capture_output=True, text=True)
        if result.returncode != 0:
            errors.append((result.stdout + result.stderr).strip())
    layer.check("S9", f"GSettings schemas compile ({count} files)", pset.fmt, errors)


def check_units(layer, pset, work: Path):
    """systemd-analyze verify on the unit files in a root that holds the whole package set, and the programs that the
    D-Bus service files start."""
    root = work / f"root-{pset.fmt}"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    for pkg in pset.packages:
        shutil.copytree(pkg.root, root, symlinks=True, dirs_exist_ok=True)
    units, errors = [], []
    for pkg in pset.packages:
        for entry in pkg.files():
            path = entry.path
            if "/systemd/" in path and path.endswith((".service", ".target", ".socket", ".timer")):
                units.append(path)
            if "/dbus-1/services/" in path and path.endswith(".service"):
                text = (root / path.lstrip("/")).read_text(errors="replace")
                match = re.search(r"^Exec=(\S+)", text, re.MULTILINE)
                if not match:
                    errors.append(f"{path} has no Exec line")
                elif match.group(1) not in pset.paths:
                    errors.append(f"{path} starts {match.group(1)}, which the package set does not hold")
    executables = {
        entry.path for pkg in pset.packages for entry in pkg.entries if entry.kind == "file" and entry.mode & 0o111
    }
    for unit in units:
        text = (root / unit.lstrip("/")).read_text(errors="replace")
        for program in re.findall(r"^Exec\w*=[-@:+!]*(/\S+)", text, re.MULTILINE):
            if program.startswith("/usr/lib/gnoblin/") and program not in executables:
                errors.append(f"{unit} starts {program}, which is not a program of the package set")
    tool = shutil.which("systemd-analyze")
    if not tool:
        errors.append("systemd-analyze is not installed on this machine")
    for unit in sorted(units) if tool else []:
        # verify reads the unit as a user unit of this machine. The programs that it starts are in the package set,
        # not on this machine, so that complaint is checked against the package set instead.
        result = subprocess.run(
            [tool, "verify", "--user", "--man=no", str(root / unit.lstrip("/"))], capture_output=True, text=True
        )
        lines = []
        for line in (result.stdout + result.stderr).splitlines():
            missing = re.search(r"Command (\S+) is not executable", line)
            if missing and missing.group(1) in executables:
                continue
            if line.strip() and not _unit_noise(line):
                lines.append(line)
        if lines:
            errors.append(f"{unit}: " + "; ".join(lines))
    if not units:
        errors.append("the package set holds no systemd unit")
    layer.check("S10", f"systemd units and D-Bus services ({len(units)} units)", pset.fmt, errors)


def _unit_noise(line: str) -> bool:
    """systemd-analyze verify reads the units of the machine that runs it as well. Lines about those are not about
    the package."""
    return "gnoblin" not in line.lower() and "portal" not in line.lower()


def check_reproducible(layer, fmt: str, files: list[Path], rebuilt: Path | None):
    if rebuilt is None:
        layer.check("S11", "reproducible", fmt, [], skip="no second build was given (--rebuilt)")
        return
    errors = []
    for path in files:
        twin = rebuilt / path.name
        if not twin.is_file():
            errors.append(f"{path.name} was not written again")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != hashlib.sha256(twin.read_bytes()).hexdigest():
            errors.append(f"{path.name} differs from the second build")
    layer.check("S11", "reproducible", fmt, errors)


def run(
    target: dict, files: list[Path], work: Path, versions: dict, release_number: str, rebuilt: Path | None
) -> Layer:
    layer = Layer("static")
    by_format: dict[str, list[Path]] = {}
    for path in files:
        by_format.setdefault(formats.format_of(path), []).append(path)
    for fmt in targets.formats(target):
        group = sorted(by_format.get(fmt, []))
        check_set(layer, fmt, group)
        packages = []
        for path in group:
            try:
                packages.append(formats.read(path, work / fmt / path.name))
            except (ValueError, OSError, subprocess.CalledProcessError) as error:
                layer.check("S2", "metadata", path.name, [f"cannot read the package: {error}"])
        if not packages:
            continue
        pset = PackageSet(fmt, packages)
        for pkg in packages:
            check_metadata(layer, target, pkg, pset, versions, release_number)
            check_dependencies(layer, target, pkg, pset)
            check_paths(layer, pkg)
            check_modes(layer, pkg, pset)
            check_links(layer, pkg, pset)
            check_build_paths(layer, pkg)
            check_checksums(layer, pkg)
            check_abi(layer, pkg, pset)
        check_desktop_files(layer, pset)
        check_schemas(layer, pset, work)
        check_units(layer, pset, work)
        check_reproducible(layer, fmt, group, rebuilt)
    return layer


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", required=True, type=Path, help="one matrix entry of scripts/supported-targets.py")
    parser.add_argument("--packages", required=True, type=Path, help="the artifact folder of the target")
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--version", default="", help="the version of the packages (default: the tree)")
    parser.add_argument("--release", default="1", help="the release number pkgsmith was given (default 1)")
    parser.add_argument("--rebuilt", type=Path, help="the same packages, written a second time")
    parser.add_argument("--work", type=Path, help="scratch folder (default: a temporary one)")
    args = parser.parse_args()
    target = json.loads(args.target.read_text())
    files = targets.select(target, args.packages)
    versions = {"gnoblin": args.version or gnoblin_version(), "portal": portal_version()}
    with tempfile.TemporaryDirectory(prefix="gnoblin-static-") as scratch:
        work = args.work or Path(scratch)
        layer = run(target, files, work, versions, args.release, args.rebuilt)
    result = layer.write(args.report)
    failed = [case for case in result["cases"] if case["status"] == "fail"]
    for case in failed:
        print(f"[static] FAIL {case['id']} {case['title']} ({case['subject']}): {case['message'].splitlines()[0]}")
    print(f"[static] {result['status']}: {len(result['cases'])} cases, {len(failed)} failed")
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
