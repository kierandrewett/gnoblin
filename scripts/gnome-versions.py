#!/usr/bin/env python3
"""Read, verify, and advance Gnoblin's GNOME release train."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "gnome-versions.json"
PROJECTS = (
    "mutter",
    "gnome-shell",
    "gnome-control-center",
    "xdg-desktop-portal-gnome",
)
UPSTREAM = {project: f"https://gitlab.gnome.org/GNOME/{project}.git" for project in PROJECTS}


def load() -> dict:
    return json.loads(MANIFEST.read_text())


def tag_commit(project: str, tag: str) -> str:
    output = subprocess.check_output(
        [
            "git",
            "ls-remote",
            "--tags",
            UPSTREAM[project],
            f"refs/tags/{tag}",
            f"refs/tags/{tag}^{{}}",
        ],
        text=True,
    )
    refs = dict(line.split("\t", 1)[::-1] for line in output.splitlines())
    return refs.get(f"refs/tags/{tag}^{{}}", refs.get(f"refs/tags/{tag}", ""))


def latest_major(project: str) -> int:
    output = subprocess.check_output(
        ["git", "ls-remote", "--tags", UPSTREAM[project], "refs/tags/*.0^{}"],
        text=True,
    )
    majors = []
    for line in output.splitlines():
        match = re.search(r"refs/tags/(\d+)\.0\^\{\}$", line)
        if match:
            majors.append(int(match.group(1)))
    if not majors:
        raise RuntimeError(f"no stable major tags found for {project}")
    return max(majors)


def generated_values(data: dict) -> tuple[tuple[Path, str, str], ...]:
    components = data["components"]
    mutter = components["mutter"]
    shell = components["gnome-shell"]
    return (
        (ROOT / "packaging/rpm/mutter.spec", r"(?m)^Version:\s+(\S+)$", mutter["version"]),
        (ROOT / "packaging/rpm/mutter.spec", r"(?m)^%global mutter_api_version\s+(\S+)$", mutter["api"]),
        (ROOT / "packaging/rpm/gnome-shell.spec", r"(?m)^Version:\s+(\S+)$", shell["version"]),
        (ROOT / "packaging/rpm/gnome-shell.spec", r"(?m)^%define mutter_version\s+(\S+)$", mutter["version"]),
        (ROOT / "flake.nix", r"mutter\.git\?rev=([0-9a-f]{40})", mutter["commit"]),
        (ROOT / "flake.nix", r"gnome-shell\.git\?rev=([0-9a-f]{40})", shell["commit"]),
        (ROOT / "src/tools/gnoblin-env.sh", r"GNOBLIN_MUTTER_API:-([^}]+)", mutter["api"]),
        (ROOT / ".github/workflows/verify.yml", r"libmutter-(\d+)\.so", mutter["api"]),
    )


def rewrite_generated(data: dict) -> None:
    for path, pattern, expected in generated_values(data):
        source = path.read_text()
        replaced, count = re.subn(
            pattern,
            lambda match: match.group(0).replace(match.group(1), expected),
            source,
            count=1,
        )
        if count != 1:
            raise RuntimeError(f"cannot update generated version field in {path.relative_to(ROOT)}")
        path.write_text(replaced)


def validate(data: dict, *, upstream: bool) -> None:
    major = int(data["major"])
    expected_tag = f"{major}.0"
    components = data["components"]
    errors = []
    for project in PROJECTS:
        component = components.get(project, {})
        if component.get("version") != expected_tag:
            errors.append(f"{project}: expected version {expected_tag}")
        commit = component.get("commit", "")
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            errors.append(f"{project}: missing 40-character release commit")
        if upstream:
            actual = tag_commit(project, expected_tag)
            if actual != commit:
                errors.append(f"{project}: {expected_tag} resolves to {actual}, not {commit}")
    if components.get("mutter", {}).get("api") != str(major):
        errors.append(f"mutter: expected API {major}")
    for path, pattern, expected in generated_values(data):
        match = re.search(pattern, path.read_text())
        if not match or match.group(1) != expected:
            errors.append(f"{path.relative_to(ROOT)}: expected generated value {expected}")
    if upstream:
        newest = min(latest_major(project) for project in PROJECTS)
        if newest > major:
            errors.append(f"GNOME {newest}.0 is available; run scripts/gnome-versions.py update {newest}")
    if errors:
        raise RuntimeError("\n".join(errors))


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    get_parser = subparsers.add_parser("get")
    get_parser.add_argument("project", choices=PROJECTS)
    get_parser.add_argument("field", choices=("version", "api", "commit"))
    check_parser = subparsers.add_parser("check")
    check_parser.add_argument("--upstream", action="store_true")
    update_parser = subparsers.add_parser("update")
    update_parser.add_argument("major", type=int)
    args = parser.parse_args()

    data = load()
    if args.command == "get":
        value = data["components"][args.project].get(args.field)
        if value is None:
            parser.error(f"{args.project} has no {args.field}")
        print(value)
        return 0
    if args.command == "check":
        validate(data, upstream=args.upstream)
        print(f"GNOME {data['major']} release manifest is consistent")
        return 0

    tag = f"{args.major}.0"
    components = {}
    for project in PROJECTS:
        commit = tag_commit(project, tag)
        if not commit:
            raise RuntimeError(f"{project} has no {tag} release tag")
        components[project] = {"version": tag, "commit": commit}
    components["mutter"]["api"] = str(args.major)
    updated = {"major": args.major, "components": components}
    MANIFEST.write_text(json.dumps(updated, indent=2) + "\n")
    rewrite_generated(updated)
    validate(updated, upstream=True)
    print(
        f"updated {MANIFEST.relative_to(ROOT)} to GNOME {args.major}; rebase patches and run just check-gnome-version"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
