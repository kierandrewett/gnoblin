#!/usr/bin/env python3
"""Make a throw-away package that only prints its own version: a test fixture for the repository tool.

usage: make-dummy-package.py rpm|deb|arch|apk NAME VERSION RELEASE OUTPUT-DIR [--dist .fc43] [--arch x86_64|amd64]

rpm needs rpmbuild, deb needs dpkg-deb and arch needs makepkg (so run it as an unprivileged user in an Arch container).
apk needs nothing: the .apk is written here, unsigned, the way pkgsmith writes it.
The package installs /usr/bin/NAME, a shell script that prints "NAME VERSION-RELEASE".
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import os
import shutil
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path


def script(name: str, version: str) -> str:
    return f"#!/bin/sh\necho '{name} {version}'\n"


def make_rpm(a: argparse.Namespace) -> None:
    top = Path(tempfile.mkdtemp(prefix="dummy-rpm-"))
    for sub in ("BUILD", "RPMS", "SOURCES", "SPECS", "SRPMS", "BUILDROOT"):
        (top / sub).mkdir()
    spec = top / "SPECS" / f"{a.name}.spec"
    spec.write_text(
        f"""%global debug_package %{{nil}}
%define _build_id_links none
Name: {a.name}
Version: {a.version}
Release: {a.release}{a.dist}
Summary: Throw-away test package
License: MIT
BuildArch: {a.arch}
%description
A test package for the repository tool. It carries no real software.
%install
mkdir -p %{{buildroot}}/usr/bin
cat > %{{buildroot}}/usr/bin/{a.name} <<'SCRIPT'
{script(a.name, f"{a.version}-{a.release}")}SCRIPT
chmod 755 %{{buildroot}}/usr/bin/{a.name}
%files
/usr/bin/{a.name}
"""
    )
    subprocess.run(["rpmbuild", "-bb", "--define", f"_topdir {top}", str(spec)], check=True, stdout=subprocess.DEVNULL)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    for rpm in (top / "RPMS").rglob("*.rpm"):
        shutil.copy(rpm, out)
    shutil.rmtree(top)


def make_deb(a: argparse.Namespace) -> None:
    top = Path(tempfile.mkdtemp(prefix="dummy-deb-"))
    (top / "usr/bin").mkdir(parents=True)
    (top / "DEBIAN").mkdir()
    binary = top / "usr/bin" / a.name
    binary.write_text(script(a.name, a.version + "-" + a.release))
    binary.chmod(0o755)
    (top / "DEBIAN/control").write_text(
        f"Package: {a.name}\nVersion: {a.version}-{a.release}\nArchitecture: {a.arch}\nMaintainer: Test <test@example.invalid>\n"
        "Section: misc\nPriority: optional\nDescription: Throw-away test package\n A test package for the repository tool.\n"
    )
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["dpkg-deb", "--root-owner-group", "--build", str(top), str(out)], check=True, stdout=subprocess.DEVNULL
    )
    shutil.rmtree(top)


def make_arch(a: argparse.Namespace) -> None:
    top = Path(tempfile.mkdtemp(prefix="dummy-arch-"))
    (top / "PKGBUILD").write_text(
        f"""pkgname={a.name}
pkgver={a.version}
pkgrel={a.release}
pkgdesc='Throw-away test package'
arch=('{a.arch}')
license=('MIT')
package() {{
    install -d "$pkgdir/usr/bin"
    printf '#!/bin/sh\\necho "{a.name} {a.version}-{a.release}"\\n' > "$pkgdir/usr/bin/{a.name}"
    chmod 755 "$pkgdir/usr/bin/{a.name}"
}}
"""
    )
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "PKGDEST": str(out)}
    subprocess.run(["makepkg", "-f", "--nodeps"], cwd=top, env=env, check=True, stdout=subprocess.DEVNULL)
    shutil.rmtree(top)


def make_apk(a: argparse.Namespace) -> None:
    """An Alpine package: a gzip stream with .PKGINFO, then one with the files. The tar of the first has no end marker."""
    body = script(a.name, f"{a.version}-r{a.release}").encode()
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for directory in ("usr", "usr/bin"):
            info = tarfile.TarInfo(directory)
            info.type, info.mode = tarfile.DIRTYPE, 0o755
            archive.addfile(info)
        info = tarfile.TarInfo(f"usr/bin/{a.name}")
        info.size, info.mode = len(body), 0o755
        info.pax_headers = {"APK-TOOLS.checksum.SHA1": hashlib.sha1(body).hexdigest()}
        archive.addfile(info, io.BytesIO(body))
    data_stream = gzip.compress(data.getvalue(), mtime=0)
    pkginfo = (
        f"pkgname = {a.name}\npkgver = {a.version}-r{a.release}\npkgdesc = Throw-away test package\n"
        f"url = https://example.invalid/\nbuilddate = {int(time.time())}\npackager = Test <test@example.invalid>\n"
        f"size = {len(body)}\narch = {a.arch}\norigin = {a.name}\nlicense = MIT\n"
        f"datahash = {hashlib.sha256(data_stream).hexdigest()}\n"
    ).encode()
    info = tarfile.TarInfo(".PKGINFO")
    info.size, info.mode = len(pkginfo), 0o644
    control = info.tobuf(tarfile.USTAR_FORMAT) + pkginfo + b"\0" * (-len(pkginfo) % 512)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / f"{a.name}-{a.version}-r{a.release}.apk").write_bytes(gzip.compress(control, mtime=0) + data_stream)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("format", choices=("rpm", "deb", "arch", "apk"))
    parser.add_argument("name")
    parser.add_argument("version")
    parser.add_argument("release")
    parser.add_argument("out")
    parser.add_argument("--dist", default="")
    parser.add_argument("--arch", default=None)
    a = parser.parse_args()
    a.arch = a.arch or {"rpm": "x86_64", "deb": "amd64", "arch": "x86_64", "apk": "x86_64"}[a.format]
    {"rpm": make_rpm, "deb": make_deb, "arch": make_arch, "apk": make_apk}[a.format](a)


if __name__ == "__main__":
    main()
