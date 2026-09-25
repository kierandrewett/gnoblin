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
PROJECT_URL = "https://github.com/kierandrewett/gnoblin"


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


def native_requirement(manifest: dict, name: str, adapter: str) -> tuple[str, str | None]:
    requirement = manifest["requirements"][name]
    return requirement["names"][adapter], requirement["minVersion"]


def render_rpm(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    next_major = manifest["release"]["gnomeMajor"] + 1
    epoch = manifest["release"]["rpmEpoch"]
    mutter_version = manifest["packages"]["gnoblin-mutter"]["version"]
    mutter_release = manifest["release"]["mutterRpmRelease"]
    packages, requirements = dependency_closure(manifest, "gnoblin")
    dependencies = [
        *(f"Requires:       {name} >= {version}" for name in packages),
        *(f"Requires:       {name} < {next_major}" for name in packages),
        *(
            f"Requires:       {package_name}" + (f" >= {minimum}" if minimum is not None else "")
            for name in requirements
            for package_name, minimum in [native_requirement(manifest, name, "rpm")]
        ),
    ]
    return (
        "# Generated from nix/native-packages.nix; do not edit.\n"
        "Name:           gnoblin\n"
        f"Version:        {version}\n"
        f"Epoch:          {epoch}\n"
        "Release:        1%{?dist}\n"
        "Summary:        Gnoblin desktop session\n"
        "License:        GPL-2.0-or-later\n"
        f"URL:            {PROJECT_URL}\n"
        "BuildArch:      noarch\n"
        + "\n".join(dependencies)
        + f"\nRequires:       gnoblin-mutter = {mutter_version}-{mutter_release}%{{?dist}}\n\n%description\n"
        "Installs the complete Gnoblin session while reusing compatible GNOME userspace.\n\n"
        "%files\n"
    )


def render_arch(manifest: dict, source_sha256: str = "SKIP") -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    gnome_version = manifest["release"]["gnomeVersion"]
    _, requirements = dependency_closure(manifest, "gnoblin")
    dependencies = [
        *(
            f"'{package_name}" + (f">={minimum}" if minimum is not None else "") + "'"
            for name in requirements
            for package_name, minimum in [native_requirement(manifest, name, "arch")]
        ),
    ]
    return (
        "# Generated from nix/native-packages.nix; do not edit.\n"
        "# shellcheck shell=bash disable=SC2034,SC2154\n"
        "pkgname=gnoblin\n"
        f"pkgver={version}\n"
        "pkgrel=1\n"
        "pkgdesc='Gnoblin desktop session with a private GNOME runtime'\n"
        "arch=('x86_64')\n"
        f"url='{PROJECT_URL}'\n"
        "license=('GPL-2.0-or-later')\n"
        "makedepends=('adwaita-cursors' 'base-devel' 'cmake' 'desktop-file-utils' 'egl-wayland' 'evolution-data-server' 'gettext' 'glib2-devel' 'gobject-introspection' 'gtk4' 'hyprcursor' 'inkscape' 'libadwaita' 'libdisplay-info' 'libei' 'libxkbcommon' 'libxkbfile' 'libxres' 'lua' 'meson' 'ninja' 'patchelf' 'pkgconf' 'python' 'python-docutils' 'python-packaging' 'sassc' 'sysprof' 'xorg-xwayland')\n"
        f"depends=({' '.join(dependencies)})\n\n"
        f'source=("$pkgname-$pkgver-gnome-{gnome_version}-arch-source.tar.xz::{PROJECT_URL}/releases/download/gnoblin-v$pkgver/$pkgname-$pkgver-gnome-{gnome_version}-arch-source.tar.xz")\n'
        f"sha256sums=('{source_sha256}')\n\n"
        "_prefix=/usr/lib/gnoblin\n"
        "\n"
        "prepare() {\n"
        '    cd "$srcdir/$pkgname-$pkgver" || return\n'
        "    for project in gsettings-desktop-schemas mutter gnome-shell; do\n"
        '        archive=(sources/"$project"-*.tar.xz)\n'
        "        test ${#archive[@]} -eq 1\n"
        '        mkdir -p "subprojects/$project"\n'
        '        tar -xf "${archive[0]}" -C "subprojects/$project" --strip-components=1\n'
        "    done\n"
        "}\n\n"
        "build() {\n"
        '    local _build_prefix="$srcdir/$pkgname-$pkgver/build-prefix"\n'
        '    cd "$srcdir/$pkgname-$pkgver" || return\n'
        '    python3 scripts/build-private-deps.py --prefix "$_build_prefix/deps" --cache "$srcdir/gnoblin-dependencies"\n'
        '    _private_pkgconfig="$_build_prefix/deps/lib64/pkgconfig:$_build_prefix/deps/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"\n'
        '    _private_gir="$_build_prefix/deps/share/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}"\n'
        '    _private_typelib="$_build_prefix/deps/lib64/girepository-1.0${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"\n'
        '    env PKG_CONFIG_PATH="$_private_pkgconfig" GI_GIR_PATH="$_private_gir" GI_TYPELIB_PATH="$_private_typelib" meson setup build/schemas subprojects/gsettings-desktop-schemas --prefix="$_prefix" --libdir=lib --buildtype=release\n'
        "    meson compile -C build/schemas\n"
        '    env PKG_CONFIG_PATH="$_private_pkgconfig" GI_GIR_PATH="$_private_gir" GI_TYPELIB_PATH="$_private_typelib" meson setup build/mutter subprojects/mutter --prefix="$_prefix" --libdir=lib --buildtype=release -Ddevkit=enabled -Dtests=disabled -Ddocs=false -Dprofiler=false -Dudev_dir="$_prefix/lib/udev"\n'
        "    meson compile -C build/mutter\n"
        '    env PKG_CONFIG_PATH="$_private_pkgconfig" GI_GIR_PATH="$_private_gir" GI_TYPELIB_PATH="$_private_typelib:$_prefix/lib/mutter-51" meson setup build/gnome-shell subprojects/gnome-shell --prefix="$_prefix" --libdir=lib --buildtype=release -Dextensions_tool=false -Dtests=false -Dman=false -Dgtk_doc=false\n'
        "    meson compile -C build/gnome-shell\n"
        "}\n\n"
        "package() {\n"
        '    local _build_prefix="$srcdir/$pkgname-$pkgver/build-prefix"\n'
        '    cd "$srcdir/$pkgname-$pkgver" || return\n'
        '    meson install -C build/schemas --destdir "$pkgdir" --no-rebuild\n'
        '    meson install -C build/mutter --destdir "$pkgdir" --no-rebuild\n'
        '    meson install -C build/gnome-shell --destdir "$pkgdir" --no-rebuild\n'
        '    rm -f "$pkgdir$_prefix/lib/systemd/user/org.gnome.Shell-disable-extensions.service"\n'
        '    install -d "$pkgdir$_prefix/deps"\n'
        '    cp -a "$_build_prefix/deps/." "$pkgdir$_prefix/deps/"\n'
        '    install -Dm644 src/data/session/modes/gnoblin.json "$pkgdir$_prefix/share/gnome-shell/modes/gnoblin.json"\n'
        '    install -Dm644 src/data/session/gnome-session/gnoblin.session "$pkgdir/usr/share/gnome-session/sessions/gnoblin.session"\n'
        '    install -Dm644 src/data/session/gnoblin.desktop "$pkgdir/usr/share/wayland-sessions/gnoblin.desktop"\n'
        "    sed -i 's|^Exec=.*|Exec=/usr/lib/gnoblin/bin/gnoblin-session|' \"$pkgdir/usr/share/wayland-sessions/gnoblin.desktop\"\n"
        '    install -Dm644 src/tools/gnoblin-env.sh "$pkgdir$_prefix/libexec/gnoblin-env.sh"\n'
        '    install -Dm755 src/tools/gnoblin-session "$pkgdir$_prefix/bin/gnoblin-session"\n'
        '    install -Dm755 src/tools/gnoblin-shell-service "$pkgdir$_prefix/bin/gnoblin-shell-service"\n'
        '    install -Dm755 src/tools/gnoblin-seed-config "$pkgdir$_prefix/libexec/gnoblin-seed-config"\n'
        '    install -Dm755 src/tools/gnoblinctl "$pkgdir/usr/bin/gnoblinctl"\n'
        '    install -Dm644 src/data/init.lua.example "$pkgdir$_prefix/share/gnoblin/init.lua.example"\n'
        '    install -Dm644 src/data/session/schemas/00_org.gnoblin.mutter.gschema.override "$pkgdir$_prefix/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override"\n'
        '    install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell.target "$pkgdir/usr/lib/systemd/user/org.gnoblin.Shell.target"\n'
        '    install -Dm644 src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf "$pkgdir/usr/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"\n'
        "    sed 's|@PREFIX@|/usr/lib/gnoblin|g' src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in >\"$pkgdir/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service\"\n"
        "    printf '%s\\n' lib >\"$pkgdir$_prefix/libexec/gnoblin-libdir\"\n"
        '    glib-compile-schemas "$pkgdir$_prefix/share/glib-2.0/schemas"\n'
        "    find \"$pkgdir$_prefix\" -type f -print0 | while IFS= read -r -d '' file; do\n"
        "        head -c 4 \"$file\" | grep -qx $'\\177ELF' || continue\n"
        "        patchelf --set-rpath '/usr/lib/gnoblin/lib:/usr/lib/gnoblin/deps/lib64' \"$file\"\n"
        "    done\n"
        "}\n"
    )


def outputs(manifest: dict) -> dict[Path, str]:
    return {
        MANIFEST_OUTPUT: render_manifest(manifest),
        ROOT / "packaging/rpm/gnoblin.spec": render_rpm(manifest),
        ROOT / "packaging/arch/PKGBUILD": render_arch(manifest),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("write", "check", "arch-release"))
    parser.add_argument("--output", type=Path, help="output path for arch-release")
    parser.add_argument("--source-sha256", help="source archive digest for arch-release")
    args = parser.parse_args()
    if args.command == "arch-release":
        if not args.output or not args.source_sha256:
            parser.error("arch-release requires --output and --source-sha256")
        if not re.fullmatch(r"[0-9a-f]{64}", args.source_sha256):
            parser.error("--source-sha256 must be a lowercase SHA-256 digest")
        # Release images do not install Nix.  The tracked manifest has already
        # been checked against Nix by the normal package-manifest gate; use it
        # here so source-package publication needs no second toolchain.
        manifest = json.loads(MANIFEST_OUTPUT.read_text())
        args.output.write_text(render_arch(manifest, args.source_sha256))
        print(f"wrote {args.output}")
        return 0

    manifest = evaluate()
    rendered_outputs = outputs(manifest)

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
