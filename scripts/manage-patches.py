#!/usr/bin/env python3
"""Export and check Gnoblin-owned patches with one mail author identity.

Usage:
  scripts/manage-patches.py check [PROJECT]
  scripts/manage-patches.py normalize [PROJECT]
  scripts/manage-patches.py export PROJECT REVISION OUTPUT [--replace]
  scripts/manage-patches.py export-worktree PROJECT OUTPUT --subject TEXT

For imported patches, keep the original author's credit in Original-Author.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
PATCH_ROOT = ROOT / "patches"
AUTHOR = "kierandrewett <kieran@drewett.dev>"
PROJECTS = ("mutter", "xdg-desktop-portal-gnome")
AUTHOR_LINE = re.compile(r"(?m)^From: [^\r\n]+$")


def paths(project: str | None) -> list[Path]:
    base = PATCH_ROOT / project if project else PATCH_ROOT
    selected = sorted(base.rglob("*.patch"))
    if project is None:
        selected += sorted((ROOT / "packaging").rglob("*.patch"))
    return selected


def normalize(content: str, source: str) -> str:
    matches = list(AUTHOR_LINE.finditer(content))
    if len(matches) != 1:
        raise ValueError(f"{source}: expected exactly one From: header, found {len(matches)}")
    return AUTHOR_LINE.sub(f"From: {AUTHOR}", content, count=1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("check", "normalize"):
        subcommand = commands.add_parser(command)
        subcommand.add_argument("project", choices=PROJECTS, nargs="?")
    export = commands.add_parser("export")
    export.add_argument("project", choices=PROJECTS)
    export.add_argument("revision")
    export.add_argument("output", type=Path)
    export.add_argument("--replace", action="store_true")
    export.add_argument("--unified", type=int, default=3)
    working = commands.add_parser("export-worktree")
    working.add_argument("project", choices=PROJECTS)
    working.add_argument("output", type=Path)
    working.add_argument("--subject", required=True)
    working.add_argument("--replace", action="store_true")
    working.add_argument("--source-tree", type=Path)
    working.add_argument("--against-index", action="store_true")
    working.add_argument(
        "--paths",
        nargs="+",
        help="limit the exported diff to paths relative to the subproject root",
    )
    args = parser.parse_args()

    if args.command in ("export", "export-worktree"):
        output = args.output.resolve()
        expected_root = (PATCH_ROOT / args.project).resolve()
        if not output.is_relative_to(expected_root) or output.suffix != ".patch":
            parser.error(f"output must be a .patch under {expected_root}")
        if output.exists() and not args.replace:
            parser.error(f"refusing to overwrite {output}; pass --replace")
        source = ROOT / "subprojects" / args.project
        if args.command == "export-worktree" and args.source_tree:
            source = args.source_tree.resolve()
        if args.command == "export":
            result = subprocess.run(
                [
                    "git",
                    "-C",
                    str(source),
                    "format-patch",
                    "-1",
                    args.revision,
                    "--stdout",
                    f"--unified={args.unified}",
                ],
                capture_output=True,
                text=True,
                check=True,
            )
            content = normalize(result.stdout, args.revision)
        else:
            result = subprocess.run(
                [
                    "git",
                    "-C",
                    str(source),
                    "diff",
                    "--binary",
                    *([] if args.against_index else ["HEAD"]),
                    "--",
                    *(args.paths or []),
                ],
                capture_output=True,
                text=True,
                check=True,
            )
            if not result.stdout:
                parser.error(f"no tracked changes in {source}")
            if "\n" in args.subject:
                parser.error("subject must be one line")
            date = subprocess.check_output(
                ["git", "-C", str(source), "show", "-s", "--format=%aD", "HEAD"],
                text=True,
            ).strip()
            content = (
                "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001\n"
                f"From: {AUTHOR}\nDate: {date}\nSubject: [PATCH] {args.subject}\n\n---\n" + result.stdout
            )
        output.parent.mkdir(parents=True, exist_ok=True)
        temporary = output.with_name(f".{output.name}.tmp")
        temporary.write_text(content)
        temporary.replace(output)
        print(output.relative_to(ROOT))
        return 0

    selected = paths(args.project)
    if not selected:
        parser.error("no Gnoblin patches found")
    changed = 0
    errors = []
    for path in selected:
        content = path.read_text()
        try:
            updated = normalize(content, str(path.relative_to(ROOT)))
        except ValueError as error:
            errors.append(str(error))
            continue
        if updated == content:
            continue
        if args.command == "check":
            errors.append(f"{path.relative_to(ROOT)}: expected From: {AUTHOR}")
        else:
            path.write_text(updated)
            changed += 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    if args.command == "normalize":
        print(f"Updated {changed} of {len(selected)} Gnoblin patches.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
