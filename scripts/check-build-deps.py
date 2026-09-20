#!/usr/bin/env python3
"""Check versioned host libraries against the pinned upstream Meson files."""

import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def requirements(source):
    variables = dict(re.findall(r"(?m)^([a-zA-Z0-9_]+)\s*=\s*'([^']+)'", source))
    for match in re.finditer(r"dependency\(\s*'([^']+)'[^)]*?version\s*:\s*([a-zA-Z0-9_]+)", source):
        module, variable = match.groups()
        minimum = variables.get(variable)
        # This schema package is built in the private prefix before Mutter.
        if minimum and module not in ("gsettings-desktop-schemas", "umockdev-1.0"):
            yield module, minimum


def check():
    if not shutil.which("pkg-config"):
        print("Missing pkg-config. Run ./build.sh --deps-only first.", file=sys.stderr)
        return 1
    versions = json.loads((ROOT / "gnome-versions.json").read_text())
    missing = set()
    for project in ("mutter", "gnome-shell"):
        revision = versions["components"][project]["commit"]
        result = subprocess.run(
            ["git", "-C", str(ROOT / "subprojects" / project), "show", f"{revision}:meson.build"],
            capture_output=True,
            text=True,
        )
        if result.returncode:
            print(f"Cannot read pinned {project} requirements. Run just init first.", file=sys.stderr)
            return 1
        for module, minimum in requirements(result.stdout):
            expression = f"{module} {minimum}"
            if subprocess.run(["pkg-config", "--exists", expression]).returncode:
                version = subprocess.run(["pkg-config", "--modversion", module], capture_output=True, text=True)
                missing.add((module, minimum, version.stdout.strip() or "not installed"))
    if missing:
        print("Build dependencies need attention:", file=sys.stderr)
        for module, minimum, installed in sorted(missing):
            print(f"  {module} {minimum}; found {installed}", file=sys.stderr)
        print(
            "Build the private dependencies with ./build.sh --deps-only, or provide them "
            "in a private prefix via PKG_CONFIG_PATH. The script does not change your release "
            "or enable testing repositories. See docs/install-source.md.",
            file=sys.stderr,
        )
        return 1
    print("Pinned GNOME library version checks passed; Meson checks remaining build requirements.")
    return 0


if __name__ == "__main__":
    sys.exit(check())
