#!/usr/bin/env python3
"""Validate native package metadata and materialize distro adapters."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST_SOURCE = ROOT / "packaging/native-packages.json"
PROJECT_URL = "https://github.com/kierandrewett/gnoblin"


def load_manifest() -> dict:
    manifest = json.loads(MANIFEST_SOURCE.read_text())
    versions = json.loads((ROOT / "gnome-versions.json").read_text())
    release = json.loads((ROOT / "gnoblin-version.json").read_text())
    metadata = manifest["release"]

    expected = {
        "gnomeMajor": versions["major"],
        "gnomeVersion": versions["components"]["mutter"]["version"],
        "gnoblinVersion": release["version"],
        "mutterApi": versions["components"]["mutter"]["api"],
    }
    for key, value in expected.items():
        if metadata[key] != value:
            raise ValueError(f"{MANIFEST_SOURCE.name}: release.{key} must be {value!r}")

    for name, source in manifest["sources"].items():
        component = versions["components"][name]
        if source != {key: component[key] for key in ("commit", "version")}:
            raise ValueError(f"{MANIFEST_SOURCE.name}: sources.{name} differs from gnome-versions.json")
    if set(manifest["sources"]) != set(versions["components"]):
        raise ValueError(f"{MANIFEST_SOURCE.name}: sources must match gnome-versions.json")
    for name, package in manifest["packages"].items():
        source_name = package.get("source")
        expected_version = versions["components"][source_name]["version"] if source_name else release["version"]
        if package["version"] != expected_version:
            raise ValueError(f"{MANIFEST_SOURCE.name}: packages.{name}.version must be {expected_version}")
    return manifest


def dependency_closure(manifest: dict, package_name: str) -> tuple[list[str], list[str], list[str]]:
    packages = manifest["packages"]
    same_major_packages: set[str] = set()
    exact_packages: set[str] = set()
    requirements: set[str] = set()

    def visit(name: str) -> None:
        package = packages[name]
        requirements.update(package.get("requires", []))
        for dependency in package.get("requiresSameMajor", []):
            if dependency not in same_major_packages:
                same_major_packages.add(dependency)
                visit(dependency)
        for dependency in package.get("requiresExact", []):
            if dependency not in exact_packages:
                exact_packages.add(dependency)
                visit(dependency)

    visit(package_name)
    exact_packages.difference_update(same_major_packages)
    return sorted(same_major_packages), sorted(exact_packages), sorted(requirements)


def native_requirement(manifest: dict, name: str, adapter: str) -> tuple[str, str | None]:
    requirement = manifest["requirements"][name]
    return requirement["names"][adapter], requirement["minVersion"]


def render_rpm(manifest: dict) -> str:
    gnoblin = manifest["packages"]["gnoblin"]
    version = gnoblin["version"]
    epoch = manifest["release"]["rpmEpoch"]
    gnoblin_release = manifest["release"]["gnoblinRpmRelease"]
    next_major = manifest["release"]["gnomeMajor"] + 1
    mutter_version = manifest["packages"]["gnoblin-mutter"]["version"]
    mutter_release = manifest["release"]["mutterRpmRelease"]
    same_major_packages, exact_packages, requirements = dependency_closure(manifest, "gnoblin")
    runtime_requirements = [
        *(f"Requires:       {name} >= {manifest['release']['gnomeMajor']}" for name in same_major_packages),
        *(f"Requires:       {name} < {next_major}" for name in same_major_packages),
        *(f"Requires:       {name} = {manifest['packages'][name]['version']}" for name in exact_packages),
        *(
            f"Requires:       {package_name}" + (f" >= {minimum}" if minimum is not None else "")
            for name in requirements
            for package_name, minimum in [native_requirement(manifest, name, "rpm")]
        ),
    ]
    integration = manifest["packages"]["gnoblin-gnome-integration"]
    integration_same_major, integration_exact, integration_native = dependency_closure(
        manifest, "gnoblin-gnome-integration"
    )
    del integration_same_major, integration_native
    integration_requires = [
        *(f"Requires:       {name} = {epoch}:{manifest['packages'][name]['version']}" for name in integration_exact),
        *(
            f"Requires:       {package_name}" + (f" >= {minimum}" if minimum is not None else "")
            for name in integration["requires"]
            for package_name, minimum in [native_requirement(manifest, name, "rpm")]
        ),
    ]
    template = (ROOT / "packaging/rpm/gnoblin.spec.in").read_text()
    replacements = {
        "@VERSION@": version,
        "@EPOCH@": str(epoch),
        "@GNOBLIN_RELEASE@": str(gnoblin_release),
        "@MUTTER_VERSION@": mutter_version,
        "@MUTTER_RELEASE@": mutter_release,
        "@RUNTIME_REQUIRES@": "\n".join(runtime_requirements),
        "@INTEGRATION_REQUIRES@": "\n".join(integration_requires),
    }
    for placeholder, value in replacements.items():
        template = template.replace(placeholder, value)
    if re.search(r"@[A-Z_]+@", template):
        raise ValueError("RPM spec template contains an unknown placeholder")
    return template


def render_arch(manifest: dict, source_sha256: str = "SKIP", release_tag: str = "gnoblin-v$pkgver") -> str:
    version = manifest["packages"]["gnoblin"]["version"]
    package_release = manifest["release"]["archPkgRelease"]
    gnome_version = manifest["release"]["gnomeVersion"]
    _, _, requirements = dependency_closure(manifest, "gnoblin")
    lua_package, lua_minimum = native_requirement(manifest, "lua", "arch")
    lua_build_requirement = lua_package + (f">={lua_minimum}" if lua_minimum is not None else "")
    dependencies = [
        f"'{package_name}" + (f">={minimum}" if minimum is not None else "") + "'"
        for name in requirements
        for package_name, minimum in [native_requirement(manifest, name, "arch")]
    ]
    # Arch package archives do not infer shared-library dependencies. These
    # are used by the private binaries even when the build tools are removed.
    dependencies += [
        f"'{name}'"
        for name in (
            "libcanberra",
            "libdisplay-info",
            "libei",
            "libnm",
            "polkit",
            "startup-notification",
        )
    ]
    return f"""# Generated from packaging/native-packages.json; do not edit.
# shellcheck shell=bash disable=SC2034,SC2154
pkgname=gnoblin
pkgver={version}
pkgrel={package_release}
pkgdesc='Standalone Gnoblin desktop session'
arch=('x86_64')
url='{PROJECT_URL}'
license=('GPL-2.0-or-later')
makedepends=('base-devel' 'cmake' 'desktop-file-utils' 'egl-wayland' 'gettext' 'glib2-devel' 'gobject-introspection' 'gtk4>=4.14.0' 'json-glib' 'libcanberra' 'libdisplay-info' 'libei' 'libnm' 'libxkbcommon' 'libxkbfile' 'libxres' 'xkeyboard-config' '{lua_build_requirement}' 'meson' 'ninja' 'patchelf' 'pkgconf' 'polkit' 'python' 'python-docutils' 'python-packaging' 'sassc' 'startup-notification' 'wayland-protocols>=1.48' 'xorg-xwayland')
depends=({" ".join(dependencies)})

source=("$pkgname-$pkgver-gnome-{gnome_version}-source.tar.xz::{PROJECT_URL}/releases/download/{release_tag}/$pkgname-$pkgver-gnome-{gnome_version}-source.tar.xz")
sha256sums=('{source_sha256}')

_prefix=/usr/lib/gnoblin

prepare() {{
    cd "$srcdir/$pkgname-$pkgver" || return
    archive=(sources/mutter-*.tar.xz)
    test ${{#archive[@]}} -eq 1
    mkdir -p subprojects/mutter
    tar -xf "${{archive[0]}}" -C subprojects/mutter --strip-components=1
}}

build() {{
    cd "$srcdir/$pkgname-$pkgver" || return
    cmake -S . -B build/ninja -G Ninja -DGNOBLIN_PREFIX="$_prefix" -DGNOBLIN_LIBDIR=lib -DGNOBLIN_BUILD_TYPE=release -DGNOBLIN_SOURCE_MODE=release-archive -DGNOBLIN_STAGE_ROOT="$srcdir/$pkgname-$pkgver/stage" -DGNOBLIN_JOBS="$(nproc)"
    cmake --build build/ninja --target gnoblin-session --parallel "$(nproc)"
}}

package() {{
    cd "$srcdir/$pkgname-$pkgver" || return
    cp -a stage/. "$pkgdir/"
    install -d "$pkgdir/usr/bin"
    ln -s "$_prefix/bin/gnoblin" "$pkgdir/usr/bin/gnoblin"
    ln -s "$_prefix/bin/gnoblinctl" "$pkgdir/usr/bin/gnoblinctl"
    install -Dm644 "$pkgdir$_prefix/share/xdg-desktop-portal/gnoblin-portals.conf" \\
        "$pkgdir/usr/share/xdg-desktop-portal/gnoblin-portals.conf"
    install -Dm644 "$pkgdir$_prefix/lib/systemd/user/gnoblin-session.target" "$pkgdir/usr/lib/systemd/user/gnoblin-session.target"
    install -Dm644 "$pkgdir$_prefix/lib/systemd/user/gnoblin-idle.service" "$pkgdir/usr/lib/systemd/user/gnoblin-idle.service"
    install -Dm644 "$pkgdir$_prefix/share/wayland-sessions/gnoblin.desktop" "$pkgdir/usr/share/wayland-sessions/gnoblin.desktop"
    sed -i -e 's|^Exec=.*|Exec=/usr/lib/gnoblin/bin/gnoblin|' -e 's|^DesktopNames=.*|DesktopNames=Gnoblin;|' "$pkgdir/usr/share/wayland-sessions/gnoblin.desktop"
    api="$(scripts/gnome-versions.py get mutter api)"
    find "$pkgdir$_prefix" -type f -print0 | while IFS= read -r -d '' file; do
        head -c 4 "$file" | grep -qx $'\177ELF' || continue
        patchelf --set-rpath "$_prefix/lib:$_prefix/lib/mutter-$api" "$file"
    done
}}
"""


def render_arch_portal(manifest: dict, source_sha256: str = "SKIP", release_tag: str | None = None) -> str:
    portal = manifest["packages"]["gnoblin-portal"]
    version = portal["version"]
    package_release = manifest["release"]["archPkgRelease"]
    gnoblin_version = manifest["packages"]["gnoblin"]["version"]
    release_tag = release_tag or f"gnoblin-v{gnoblin_version}"
    requirements = [
        f"'{native_requirement(manifest, name, 'arch')[0]}"
        + (
            f">={native_requirement(manifest, name, 'arch')[1]}"
            if native_requirement(manifest, name, "arch")[1] is not None
            else ""
        )
        + "'"
        for name in portal["requires"]
    ]
    return f"""# Generated from packaging/native-packages.json; do not edit.
# shellcheck shell=bash disable=SC2034,SC2154
pkgname=gnoblin-portal
pkgver={version}
pkgrel={package_release}
pkgdesc='Optional GTK-based portal backend for Gnoblin sessions'
arch=('x86_64')
url='{PROJECT_URL}'
license=('LGPL-2.1-or-later')
makedepends=('base-devel' 'gettext' 'glib2-devel' 'glycin' 'gsettings-desktop-schemas' 'gtk4>=4.22.0' 'libadwaita' 'meson' 'ninja' 'pkgconf' 'xdg-desktop-portal>=1.21.1')
depends=({" ".join(requirements)} 'glycin' 'libadwaita' 'libsecret')

source=("xdg-desktop-portal-gnome-$pkgver.tar.xz::{PROJECT_URL}/releases/download/{release_tag}/xdg-desktop-portal-gnome-$pkgver.tar.xz")
sha256sums=('{source_sha256}')

_prefix=/usr/lib/gnoblin

build() {{
    cd "$srcdir/xdg-desktop-portal-gnome-$pkgver" || return
    meson setup build --prefix="$_prefix" --libdir=lib --libexecdir=libexec \\
        --datadir=share -Ddbus_service_dir="$_prefix/share/dbus-1/services" \\
        -Dsystemduserunitdir="$_prefix/lib/systemd/user" --wrap-mode=nodownload
    meson compile -C build
}}

package() {{
    cd "$srcdir/xdg-desktop-portal-gnome-$pkgver" || return
    DESTDIR="$pkgdir" meson install -C build --no-rebuild
    install -Dm644 "$pkgdir$_prefix/share/xdg-desktop-portal/portals/gnoblin.portal" \\
        "$pkgdir/usr/share/xdg-desktop-portal/portals/gnoblin.portal"
    install -Dm644 "$pkgdir$_prefix/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service" \\
        "$pkgdir/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service"
    install -Dm644 "$pkgdir$_prefix/lib/systemd/user/xdg-desktop-portal-gnoblin.service" \\
        "$pkgdir/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service"
}}
"""


def render_arch_integration(manifest: dict) -> str:
    version = manifest["packages"]["gnoblin-gnome-integration"]["version"]
    requirements = manifest["packages"]["gnoblin-gnome-integration"]["requires"]
    dependencies = [
        "'gnoblin'",
        *(f"'{native_requirement(manifest, name, 'arch')[0]}'" for name in requirements),
    ]
    return f"""# Generated from packaging/native-packages.json; do not edit.
# shellcheck shell=bash disable=SC2034
pkgname=gnoblin-gnome-integration
pkgver={version}
pkgrel=1
pkgdesc='Optional GNOME application services for Gnoblin'
arch=('any')
url='{PROJECT_URL}'
license=('GPL-2.0-or-later')
depends=({" ".join(dependencies)})

package() {{
    :
}}
"""


def outputs(manifest: dict) -> dict[Path, str]:
    return {
        ROOT / "packaging/rpm/gnoblin.spec": render_rpm(manifest),
        ROOT / "packaging/arch/PKGBUILD": render_arch(manifest),
        ROOT / "packaging/arch/portal/PKGBUILD": render_arch_portal(manifest),
        ROOT / "packaging/arch/gnome-integration/PKGBUILD": render_arch_integration(manifest),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("write", "check", "arch-release", "arch-portal-release"))
    parser.add_argument("--output", type=Path, help="output path for arch-release")
    parser.add_argument("--source-sha256", help="source archive digest for arch-release")
    parser.add_argument("--release-tag", help="GitHub release tag for arch-release")
    args = parser.parse_args()
    if args.command == "arch-release":
        if not args.output or not args.source_sha256:
            parser.error("arch-release requires --output and --source-sha256")
        if not re.fullmatch(r"[0-9a-f]{64}", args.source_sha256):
            parser.error("--source-sha256 must be a lowercase SHA-256 digest")
        manifest = load_manifest()
        args.output.write_text(
            render_arch(
                manifest,
                args.source_sha256,
                args.release_tag or "gnoblin-v$pkgver",
            )
        )
        print(f"wrote {args.output}")
        return 0

    if args.command == "arch-portal-release":
        if not args.output or not args.source_sha256:
            parser.error("arch-portal-release requires --output and --source-sha256")
        if not re.fullmatch(r"[0-9a-f]{64}", args.source_sha256):
            parser.error("--source-sha256 must be a lowercase SHA-256 digest")
        manifest = load_manifest()
        args.output.write_text(
            render_arch_portal(
                manifest,
                args.source_sha256,
                args.release_tag,
            )
        )
        print(f"wrote {args.output}")
        return 0

    manifest = load_manifest()
    rendered_outputs = outputs(manifest)

    if args.command == "write":
        for output, rendered in rendered_outputs.items():
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered)
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
    print("native package outputs match packaging/native-packages.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
