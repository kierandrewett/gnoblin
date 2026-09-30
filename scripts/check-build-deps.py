#!/usr/bin/env python3
"""Check host libraries required by the pinned upstream Meson files."""

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
MESON_ASSIGNMENT = re.compile(r"^([a-zA-Z0-9_]+)\s*=\s*'([^']+)'$", re.MULTILINE)


def bundled_subprojects(source):
    return set(re.findall(r"\bsubproject\(\s*['\"]([^'\"]+)", source))


def requirements(source, include_schemas=False, bundled=()):
    variables = dict(re.findall(r"(?m)^([a-zA-Z0-9_]+)\s*=\s*'([^']+)'", source))
    subprojects = bundled_subprojects(source) | set(bundled)
    for match in re.finditer(r"\bdependency\s*\(", source):
        start = match.end()
        depth = 1
        quote = None
        escaped = False
        for end in range(start, len(source)):
            character = source[end]
            if quote:
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == quote:
                    quote = None
            elif character in "'\"":
                quote = character
            elif character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    break
        else:
            continue

        arguments = source[start:end]
        module_match = re.match(r"\s*(['\"])([^'\"]+)\1", arguments)
        if not module_match:
            continue
        module = module_match.group(2)
        if re.search(r"\brequired\s*:\s*false\b", arguments):
            continue
        version_match = re.search(r"\bversion\s*:\s*('[^']+'|[a-zA-Z0-9_]+)", arguments)
        expression = version_match.group(1) if version_match else None
        minimum = (
            (expression[1:-1] if expression.startswith("'") else variables.get(expression)) if expression else None
        )
        # The pinned release version is checked once below. Upstream source
        # files may declare several earlier prerelease floors for this module.
        if module not in subprojects | {"umockdev-1.0"} and (include_schemas or module != "gsettings-desktop-schemas"):
            yield module, minimum


def apply_dependency_version_patches(source, project, build_file):
    """Apply patched Meson version-variable assignments to pinned source text."""
    variables = dict(MESON_ASSIGNMENT.findall(source))
    patch_root = ROOT / "patches" / project

    def apply_section(target, removed, added, patch):
        if target != build_file:
            return
        for name, new_value in added.items():
            old_value = removed.get(name)
            if old_value is None:
                continue
            current_value = variables.get(name)
            if current_value not in (old_value, new_value):
                raise ValueError(
                    f"{patch}: expected {name} = {old_value!r} in pinned {project}/{build_file}, "
                    f"found {current_value!r}"
                )
            variables[name] = new_value

    for patch in sorted(patch_root.rglob("*.patch")):
        target = None
        removed = {}
        added = {}
        for line in patch.read_text().splitlines():
            if line.startswith("diff --git "):
                apply_section(target, removed, added, patch)
                parts = line.split()
                target = parts[3][2:] if len(parts) == 4 and parts[3].startswith("b/") else None
                removed = {}
                added = {}
            elif target == build_file and line.startswith(("-", "+")) and not line.startswith(("---", "+++")):
                match = MESON_ASSIGNMENT.fullmatch(line[1:])
                if match:
                    assignments = removed if line.startswith("-") else added
                    assignments[match.group(1)] = match.group(2)
        apply_section(target, removed, added, patch)

    def replace(match):
        name = match.group(1)
        return f"{name} = '{variables[name]}'"

    return re.sub(r"(?m)^([a-zA-Z0-9_]+)\s*=\s*'[^']+'$", replace, source)


def check(mode="checkout", project=None, xwayland=True, vector_cursors=False):
    if not shutil.which("pkg-config"):
        print("Missing pkg-config. Install your distribution's development tools.", file=sys.stderr)
        return 1
    projects = {
        "mutter": "meson.build",
        "xdg-desktop-portal-gnome": "src/meson.build",
    }
    # Match the Meson options used by the source build. This preflight reads
    # version declarations, including ones inside disabled feature branches.
    disabled_modules = {
        "mutter": {"gtk+-3.0", "sysprof-6", "sysprof-capture-4"},
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
    if project is None or project == "mutter":
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
            root_source = (ROOT / "subprojects" / source_project / "meson.build").read_text()
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
            if build_file == "meson.build":
                root_source = source
            else:
                result = subprocess.run(
                    ["git", "-C", str(ROOT / "subprojects" / source_project), "show", f"{revision}:meson.build"],
                    capture_output=True,
                    text=True,
                )
                if result.returncode:
                    print(
                        f"Cannot read pinned {source_project} subprojects. Check the source checkout.", file=sys.stderr
                    )
                    return 1
                root_source = result.stdout
            source = apply_dependency_version_patches(source, source_project, build_file)
        for module, minimum in requirements(
            source,
            include_schemas=(source_project == "xdg-desktop-portal-gnome" and project is not None),
            bundled=bundled_subprojects(root_source),
        ):
            if module in disabled_modules.get(source_project, set()):
                continue
            expression = (
                module if minimum is None else f"{module} {minimum if not minimum[0].isdigit() else '>=' + minimum}"
            )
            if subprocess.run(["pkg-config", "--exists", expression]).returncode:
                version = subprocess.run(["pkg-config", "--modversion", module], capture_output=True, text=True)
                missing.add((module, minimum or "required", version.stdout.strip() or "not installed"))
        if source_project == "mutter":
            if subprocess.run(["pkg-config", "--exists", "xkeyboard-config"]).returncode:
                version = subprocess.run(
                    ["pkg-config", "--modversion", "xkeyboard-config"], capture_output=True, text=True
                )
                missing.add(("xkeyboard-config", "required", version.stdout.strip() or "not installed"))
    if missing:
        print("Build dependencies need attention:", file=sys.stderr)
        for module, minimum, installed in sorted(missing):
            print(f"  {module} {minimum}; found {installed}", file=sys.stderr)
        print("Install development packages at these versions or newer. See docs/install-source.md.", file=sys.stderr)
        return 1
    print("Pinned GNOME dependency checks passed; Meson checks remaining build requirements.")
    return 0


if __name__ == "__main__":
    default_mode = "checkout" if (ROOT / ".git").exists() else "release-archive"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", nargs="?", default=default_mode, choices=("checkout", "release-archive"))
    parser.add_argument("project", nargs="?", choices=("mutter", "xdg-desktop-portal-gnome"))
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
