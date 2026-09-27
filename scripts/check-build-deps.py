#!/usr/bin/env python3
"""Check versioned host libraries against the pinned upstream Meson files."""

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def requirements(source, include_schemas=False):
    variables = dict(re.findall(r"(?m)^([a-zA-Z0-9_]+)\s*=\s*'([^']+)'", source))
    for match in re.finditer(
        r"dependency\(\s*'([^']+)'[^)]*?version\s*:\s*('[^']+'|[a-zA-Z0-9_]+)",
        source,
    ):
        module, expression = match.groups()
        minimum = expression[1:-1] if expression.startswith("'") else variables.get(expression)
        # The pinned release version is checked once below. Upstream source
        # files may declare several earlier prerelease floors for this module.
        if (
            minimum
            and module not in ("umockdev-1.0", "libgxdp")
            and (include_schemas or module != "gsettings-desktop-schemas")
        ):
            yield module, minimum


def check(mode="checkout", project=None, xwayland=True, vector_cursors=False):
    if not shutil.which("pkg-config"):
        print("Missing pkg-config. Install your distribution's development tools.", file=sys.stderr)
        return 1
    projects = {
        "mutter": "meson.build",
        "gnome-shell": "meson.build",
        "xdg-desktop-portal-gnome": "src/meson.build",
    }
    # Match the Meson options used by the source build. This preflight reads
    # version declarations, including ones inside disabled feature branches.
    disabled_modules = {
        "mutter": {"gtk+-3.0", "sysprof-capture-4"},
        "gnome-shell": {"libecal-2.0", "libedataserver-1.2"},
    }
    if not vector_cursors:
        disabled_modules["mutter"].add("hyprcursor")
    if not xwayland:
        # These Mutter dependencies sit inside its XWayland-only Meson branch.
        disabled_modules["mutter"].update(
            {
                "x11",
                "xcomposite",
                "xfixes",
                "xi",
                "xrandr",
                "libstartup-notification-1.0",
            }
        )
    if project is not None and project not in projects:
        print(f"Unknown source project: {project}", file=sys.stderr)
        return 2
    if mode == "release-archive":
        prepare = subprocess.run(
            [
                str(ROOT / "scripts/prepare-build-sources.sh"),
                mode,
                *([project] if project else []),
            ]
        )
        if prepare.returncode:
            return prepare.returncode
    versions = json.loads((ROOT / "gnome-versions.json").read_text())
    missing = set()
    if project is None or project in ("mutter", "gnome-shell"):
        schemas_version = versions["components"]["gsettings-desktop-schemas"]["version"]
        if subprocess.run(["pkg-config", "--exists", f"gsettings-desktop-schemas >= {schemas_version}"]).returncode:
            version = subprocess.run(
                ["pkg-config", "--modversion", "gsettings-desktop-schemas"], capture_output=True, text=True
            )
            missing.add(
                ("gsettings-desktop-schemas", f">= {schemas_version}", version.stdout.strip() or "not installed")
            )
    for source_project in (project,) if project else projects:
        build_file = projects[source_project]
        if mode == "release-archive":
            source = (ROOT / "subprojects" / source_project / build_file).read_text()
        else:
            revision = versions["components"][source_project]["commit"]
            result = subprocess.run(
                ["git", "-C", str(ROOT / "subprojects" / source_project), "show", f"{revision}:{build_file}"],
                capture_output=True,
                text=True,
            )
            if result.returncode:
                print(f"Cannot read pinned {source_project} requirements. Check the source checkout.", file=sys.stderr)
                return 1
            source = result.stdout
        for module, minimum in requirements(
            source, include_schemas=(source_project == "xdg-desktop-portal-gnome" and project is not None)
        ):
            if module in disabled_modules.get(source_project, set()):
                continue
            expression = f"{module} {minimum if not minimum[0].isdigit() else '>=' + minimum}"
            if subprocess.run(["pkg-config", "--exists", expression]).returncode:
                version = subprocess.run(["pkg-config", "--modversion", module], capture_output=True, text=True)
                missing.add((module, minimum, version.stdout.strip() or "not installed"))
    if missing:
        print("Build dependencies need attention:", file=sys.stderr)
        for module, minimum, installed in sorted(missing):
            print(f"  {module} {minimum}; found {installed}", file=sys.stderr)
        print("Install development packages at these versions or newer. See docs/install-source.md.", file=sys.stderr)
        return 1
    if project is None or project == "gnome-shell":
        gjs = shutil.which("gjs")
        if not gjs:
            print("Missing gjs runtime. Install your distribution's gjs package.", file=sys.stderr)
            return 1
        runtime_typelibs = []
        for namespace in ("AccountsService", "IBus"):
            probe = subprocess.run(
                [
                    gjs,
                    "-c",
                    f"imports.gi.versions.{namespace} = '1.0'; imports.gi.{namespace};",
                ],
                capture_output=True,
                text=True,
            )
            if probe.returncode:
                runtime_typelibs.append(f"{namespace}-1.0")
        if runtime_typelibs:
            print("Shell runtime libraries need attention:", file=sys.stderr)
            for namespace in runtime_typelibs:
                print(f"  {namespace} typelib could not load", file=sys.stderr)
            print(
                "Install your distribution's GObject introspection runtime packages. See docs/install-source.md.",
                file=sys.stderr,
            )
            return 1
    print("Pinned GNOME library version checks passed; Meson checks remaining build requirements.")
    return 0


if __name__ == "__main__":
    default_mode = "checkout" if (ROOT / ".git").exists() else "release-archive"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", nargs="?", default=default_mode, choices=("checkout", "release-archive"))
    parser.add_argument("project", nargs="?", choices=("mutter", "gnome-shell", "xdg-desktop-portal-gnome"))
    parser.add_argument("--without-xwayland", action="store_true")
    parser.add_argument("--with-vector-cursors", action="store_true")
    arguments = parser.parse_args()
    sys.exit(
        check(
            arguments.mode,
            arguments.project,
            xwayland=not arguments.without_xwayland,
            vector_cursors=arguments.with_vector_cursors,
        )
    )
