#!/usr/bin/env python3
"""Package a runtime built at /usr/lib/gnoblin inside a Debian/Ubuntu build container."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PREFIX = Path("/usr/lib/gnoblin")
PUBLIC_FILES = (
    "share/wayland-sessions/gnoblin.desktop",
    "share/gnome-session/sessions/gnoblin.session",
    "share/gnoblin/init.lua.example",
    "lib/systemd/user/org.gnoblin.Shell.target",
    "lib/systemd/user/org.gnoblin.Shell@wayland.service",
    "lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf",
)
SERVICES = (
    "gnome-session-bin",
    "gnome-session-common",
    "gnome-settings-daemon",
    "xdg-desktop-portal-gnome",
    "python3",
    "python3-gi",
    "gir1.2-gtk-3.0",
    "dbus-user-session",
    "systemd",
    "xwayland",
    "gsettings-desktop-schemas",
    "dconf-gsettings-backend",
    "iso-codes",
    "adwaita-icon-theme",
    "bubblewrap",
    "wireplumber",
    "playerctl",
    "brightnessctl",
    "libglib2.0-bin",
)


def output(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def package_version(gnome_version, gnoblin_version, revision, distribution, release):
    """Encode the GNOME compatibility train and Gnoblin's SemVer identity."""
    return f"{gnome_version}+gnoblin{gnoblin_version}-{revision}~{distribution}{release}"


def elf_files(directory):
    for path in sorted(directory.rglob("*")):
        if path.is_file() and not path.is_symlink():
            with path.open("rb") as stream:
                header = stream.read(18)
            if len(header) == 18 and header[:4] == b"\x7fELF":
                kind = int.from_bytes(header[16:18], "little" if header[5] == 1 else "big")
                if kind in (2, 3):
                    yield path


def stage_runtime(prefix, stage):
    """Export only Gnoblin-named entry points; keep upstream files private."""
    for relative in (
        "bin/gnome-shell",
        "bin/gnoblin-session",
        "libexec/gnoblin-seed-config",
        "share/gnoblin/init.lua.example",
        "deps/lib64",
        *PUBLIC_FILES,
    ):
        if not (prefix / relative).exists():
            raise RuntimeError(f"Incomplete private runtime: {prefix / relative}")
    private = stage / PREFIX.relative_to("/")
    shutil.copytree(prefix, private, symlinks=True)
    # Development files are not needed to run the compositor.
    for base in (private, private / "deps"):
        for relative in ("include", "share/gir-1.0", "lib64/pkgconfig", "share/pkgconfig", ".gnoblin-dependencies"):
            shutil.rmtree(base / relative, ignore_errors=True)
        (base / ".build.lock").unlink(missing_ok=True)
        for path in base.rglob("*.a"):
            path.unlink()
    for relative in PUBLIC_FILES:
        target = stage / "usr" / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(private / relative, target)
    command = stage / "usr/bin/gnoblinctl"
    command.parent.mkdir(parents=True, exist_ok=True)
    command.symlink_to("../lib/gnoblin/bin/gnoblinctl")
    policy = private / "share/polkit-1/actions/org.gnome.mutter.backlight-helper.policy"
    if policy.exists():
        target = stage / "usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(policy.read_text().replace("org.gnome.mutter", "org.gnoblin.mutter"))
    return private


def typelib_dependencies(private):
    packages = set()
    for path in private.rglob("*.typelib"):
        if not path.is_symlink():
            continue
        target = path.resolve(strict=True)
        if target.is_relative_to(PREFIX):
            continue
        owners = output("dpkg-query", "-S", str(target)).splitlines()
        for owner in owners:
            package = owner.rsplit(": ", 1)[0]
            version = output("dpkg-query", "-W", "-f=${Version}", package)
            packages.add(f"{package} (>= {version})")
    return packages


def library_dependencies(private, work):
    """Use Debian's symbol database for system libraries, excluding bundled ones."""
    binaries = list(elf_files(private))
    if not binaries:
        raise RuntimeError("Runtime contains no ELF binaries")
    local = set()
    directories = set()
    for binary in binaries:
        soname = output("patchelf", "--print-soname", str(binary))
        match = re.fullmatch(r"(.+)\.so\.(.+)", soname) or re.fullmatch(r"(.+)-([0-9][^/]*)\.so", soname)
        if match:
            local.add(f"{match[1]} {match[2]} gnoblin")
        directories.add(str(binary.parent))
        rpath = output("patchelf", "--print-rpath", str(binary))
        if "/tmp/" in rpath or "/build/" in rpath or "/home/" in rpath:
            raise RuntimeError(f"Build path in runtime library search path: {binary}: {rpath}")
    (work / "debian").mkdir(exist_ok=True)
    (work / "debian/control").write_text("Source: gnoblin\n\nPackage: gnoblin\nArchitecture: any\n")
    (work / "debian/shlibs.local").write_text("\n".join(sorted(local)) + "\n")
    dependencies = output(
        "dpkg-shlibdeps",
        "-O",
        "-xgnoblin",
        f"-S{work / 'debian/gnoblin'}",
        *(f"-l{path}" for path in sorted(directories)),
        *(f"-e{path}" for path in binaries),
        cwd=work,
    )
    return {
        item
        for line in dependencies.splitlines()
        if line.startswith("shlibs:Depends=")
        for item in line.removeprefix("shlibs:Depends=").split(", ")
        if item
    }


def check_layout(stage):
    public = {Path("usr") / name for name in PUBLIC_FILES}
    public.update(
        (Path("usr/bin/gnoblinctl"), Path("usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"))
    )
    for path in stage.rglob("*"):
        if path.is_dir() and not path.is_symlink():
            continue
        relative = path.relative_to(stage)
        if not (
            relative.is_relative_to("usr/lib/gnoblin")
            or relative.is_relative_to("usr/share/doc/gnoblin")
            or relative.is_relative_to("DEBIAN")
            or relative in public
        ):
            raise RuntimeError(f"File outside Gnoblin package layout: {relative}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/deb")
    parser.add_argument("--revision", default="1")
    args = parser.parse_args()
    if not re.fullmatch(r"[1-9][0-9]*", args.revision):
        parser.error("revision must be a positive integer")
    distro = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines() if "=" in line)
    distribution = distro["ID"].strip('"')
    release = distro["VERSION_ID"].strip('"')
    if distribution not in ("debian", "ubuntu") or not re.fullmatch(r"[0-9.]+", release):
        parser.error("Build in a released Debian or Ubuntu container")
    gnome_version = json.loads((ROOT / "gnome-versions.json").read_text())["components"]["gnome-shell"]["version"]
    gnoblin_version = json.loads((ROOT / "gnoblin-version.json").read_text())["version"]
    commit = output("git", "rev-parse", "HEAD", cwd=ROOT)
    version = package_version(gnome_version, gnoblin_version, args.revision, distribution, release)
    arch = output("dpkg", "--print-architecture")
    args.output.mkdir(parents=True, exist_ok=True)
    artifact = args.output.resolve() / f"gnoblin-{distribution}{release}-{arch}.deb"
    if artifact.exists():
        parser.error(f"Refusing to overwrite {artifact}")
    with tempfile.TemporaryDirectory(prefix="gnoblin-deb-") as temporary:
        work = Path(temporary)
        stage = work / "debian/gnoblin"
        private = stage_runtime(PREFIX, stage)
        info = stage / "DEBIAN"
        info.mkdir()
        (info / "control").write_text("Package: gnoblin\nVersion: 1\nArchitecture: any\n")
        dependencies = sorted(set(SERVICES) | typelib_dependencies(private) | library_dependencies(private, work))
        size = sum(path.stat().st_size for path in stage.rglob("*") if path.is_file() and not path.is_symlink())
        (info / "control").write_text(
            f"Package: gnoblin\nVersion: {version}\nArchitecture: {arch}\n"
            "Maintainer: Gnoblin Developers <release@gnoblin.local>\n"
            "Section: x11\nPriority: optional\n"
            f"Installed-Size: {(size + 1023) // 1024}\nDepends: {', '.join(dependencies)}\n"
            "Homepage: https://github.com/kierandrewett/gnoblin\n"
            "Description: Wayland desktop with a private GNOME runtime\n"
            " Installs a separate Gnoblin login session without replacing GNOME.\n"
        )
        docs = stage / "usr/share/doc/gnoblin"
        docs.mkdir(parents=True)
        shutil.copy2(ROOT / "build-dependencies.json", docs / "build-dependencies.json")
        (docs / "build-info.json").write_text(
            json.dumps(
                {
                    "commit": commit,
                    "gnoblinVersion": gnoblin_version,
                    "gnomeVersion": gnome_version,
                    "distribution": distribution,
                    "release": release,
                    "architecture": arch,
                    "source": f"https://github.com/kierandrewett/gnoblin/tree/{commit}",
                },
                indent=2,
            )
            + "\n"
        )
        shutil.copy2(ROOT / "COPYING", docs / "copyright")
        for source in [
            *(ROOT / "subprojects").glob("*"),
            *(ROOT / "build/dependencies").glob("*/source"),
            *(ROOT / "build/dependencies").glob("*/build/cargo-home/registry/src/*/*"),
        ]:
            if not source.is_dir():
                continue
            name = source.parent.name if source.name == "source" else source.name
            for notice in source.iterdir():
                if notice.is_file() and notice.name.upper().startswith(
                    ("COPYING", "LICENSE", "LICENCE", "NOTICE", "AUTHORS", "COPYRIGHT")
                ):
                    target = docs / "licenses" / name / notice.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(notice, target)
        manifest = ROOT / "build/deb-dependencies.json"
        if manifest.exists():
            shutil.copy2(manifest, docs / "build-dependencies.json")
        (info / "md5sums").write_text(
            "".join(
                f"{hashlib.md5(path.read_bytes()).hexdigest()}  {path.relative_to(stage)}\n"
                for path in sorted(stage.rglob("*"))
                if path.is_file() and not path.is_symlink() and not path.is_relative_to(info)
            )
        )
        check_layout(stage)
        subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(stage), str(artifact)], check=True)
    with artifact.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    artifact.with_suffix(".deb.sha256").write_text(f"{digest}  {artifact.name}\n")
    print(artifact)


if __name__ == "__main__":
    main()
