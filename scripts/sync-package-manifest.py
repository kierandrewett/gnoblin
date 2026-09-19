#!/usr/bin/env python3
"""Materialize the Nix-owned native-package interface for distro adapters."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST_OUTPUT = ROOT / "packaging/generated/manifest.json"


def evaluate() -> dict:
    result = subprocess.run(
        ["nix", "eval", "--json", ".#lib.nativePackages"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def render_manifest(manifest: dict) -> str:
    rendered = json.dumps(manifest, indent=2, sort_keys=True)

    # Match Prettier's compact representation for short arrays of strings so
    # the generated snapshot also passes the repository-wide formatter check.
    def compact_strings(match: re.Match) -> str:
        values = [line.strip().removesuffix(",") for line in match.group(1).splitlines()]
        compact = "[" + ", ".join(values) + "]"
        return compact if len(compact) <= 120 else match.group(0)

    rendered = re.sub(r"\[\n((?:\s+\"[^\n]+\",?\n)+)\s+\]", compact_strings, rendered)
    return rendered + "\n"


def dependency_closure(manifest: dict, package_name: str) -> tuple[list[str], list[str]]:
    packages = manifest["packages"]
    package_names: set[str] = set()
    requirements: set[str] = set()

    def visit(name: str) -> None:
        package = packages[name]
        requirements.update(package.get("requires", []))
        for dependency in package.get("requiresSameMajor", []):
            if dependency not in package_names:
                package_names.add(dependency)
                visit(dependency)

    visit(package_name)
    return sorted(package_names), sorted(requirements)


def render_rpm(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    next_major = manifest["release"]["gnomeMajor"] + 1
    packages, requirements = dependency_closure(manifest, "gnoblin")
    dependencies = [
        *(f"Requires:       {name} >= {version}" for name in packages),
        *(f"Requires:       {name} < {next_major}" for name in packages),
        *(
            f"Requires:       {manifest['requirements'][name]['names']['rpm']} >= "
            f"{manifest['requirements'][name]['minVersion']}"
            for name in requirements
        ),
    ]
    return (
        "# Generated from nix/native-packages.nix; do not edit.\n"
        "Name:           gnoblin\n"
        f"Version:        {version}\n"
        "Release:        1%{?dist}\n"
        "Summary:        Gnoblin desktop session\n"
        "License:        GPL-2.0-or-later\n"
        "URL:            https://github.com/kdrew7/gnoblin\n"
        "BuildArch:      noarch\n"
        + "\n".join(dependencies)
        + "\n\n%description\n"
        "Installs the complete Gnoblin session while reusing compatible GNOME userspace.\n\n"
        "%files\n"
    )


def render_debian_control(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    next_major = manifest["release"]["gnomeMajor"] + 1
    packages, requirements = dependency_closure(manifest, "gnoblin")
    dependencies = [
        *(f"{name} (>= {version})" for name in packages),
        *(f"{name} (<< {next_major})" for name in packages),
        *(
            f"{manifest['requirements'][name]['names']['deb']} "
            f"(>= {manifest['requirements'][name]['minVersion']})"
            for name in requirements
        ),
    ]
    wrapped = ",\n         ".join(dependencies)
    return (
        "# Generated from nix/native-packages.nix; do not edit.\n"
        "Source: gnoblin\n"
        "Section: gnome\n"
        "Priority: optional\n"
        "Maintainer: Gnoblin Developers <gnoblin@example.invalid>\n"
        "Build-Depends: debhelper-compat (= 13)\n"
        "Standards-Version: 4.7.2\n"
        "Rules-Requires-Root: no\n\n"
        "Package: gnoblin\n"
        "Architecture: all\n"
        f"Depends: {wrapped}\n"
        "Description: Gnoblin desktop session\n"
        " Installs the complete Gnoblin session while reusing compatible GNOME userspace.\n"
    )


def render_arch(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    next_major = manifest["release"]["gnomeMajor"] + 1
    packages, requirements = dependency_closure(manifest, "gnoblin")
    dependencies = [
        *(f"'{name}>={version}'" for name in packages),
        *(f"'{name}<{next_major}'" for name in packages),
        *(
            f"'{manifest['requirements'][name]['names']['arch']}"
            f">={manifest['requirements'][name]['minVersion']}'"
            for name in requirements
        ),
    ]
    return (
        "# Generated from nix/native-packages.nix; do not edit.\n"
        "pkgname=gnoblin\n"
        f"pkgver={version}\n"
        "pkgrel=1\n"
        "pkgdesc='Gnoblin desktop session'\n"
        "arch=('any')\n"
        "url='https://github.com/kdrew7/gnoblin'\n"
        "license=('GPL-2.0-or-later')\n"
        f"depends=({' '.join(dependencies)})\n\n"
        "package() {\n"
        "    install -dm755 \"$pkgdir/usr/share/gnoblin\"\n"
        "}\n"
    )


def render_debian_changelog(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    return (
        f"gnoblin ({version}-1) unstable; urgency=medium\n\n"
        "  * Generate native metapackage from the Nix package model.\n\n"
        " -- Gnoblin Developers <gnoblin@example.invalid>  Thu, 01 Jan 1970 00:00:00 +0000\n"
    )


def outputs(manifest: dict) -> dict[Path, str]:
    return {
        MANIFEST_OUTPUT: render_manifest(manifest),
        ROOT / "packaging/rpm/gnoblin.spec": render_rpm(manifest),
        ROOT / "packaging/deb/debian/control": render_debian_control(manifest),
        ROOT / "packaging/deb/debian/changelog": render_debian_changelog(manifest),
        ROOT / "packaging/deb/debian/rules": "#!/usr/bin/make -f\n%:\n\tdh $@\n",
        ROOT / "packaging/deb/debian/source/format": "3.0 (native)\n",
        ROOT / "packaging/deb/debian/copyright": (
            "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n"
            "Upstream-Name: gnoblin\n"
            "Source: https://github.com/kdrew7/gnoblin\n\n"
            "Files: *\nCopyright: Gnoblin Developers\nLicense: GPL-2+\n"
        ),
        ROOT / "packaging/arch/PKGBUILD": render_arch(manifest),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("write", "check"))
    args = parser.parse_args()
    rendered_outputs = outputs(evaluate())

    if args.command == "write":
        for output, rendered in rendered_outputs.items():
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered)
            if output.name == "rules":
                output.chmod(0o755)
            print(f"wrote {output.relative_to(ROOT)}")
        return 0

    stale = [
        output.relative_to(ROOT)
        for output, rendered in rendered_outputs.items()
        if not output.exists() or output.read_text() != rendered
    ]
    if stale:
        print("native package outputs are stale:", file=sys.stderr)
        for output in stale:
            print(f"  {output}", file=sys.stderr)
        print("run scripts/sync-package-manifest.py write", file=sys.stderr)
        return 1
    print("native package outputs match Nix")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
