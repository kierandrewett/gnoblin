#!/usr/bin/env python3
"""Export and check Gnoblin-owned patches with one mail author identity.

patches/<project>/series lists the patches in apply order and is the only source of that order. The patch files sit
in topic directories, <NN>-<topic>/<NNNN>-<slug>.patch. Both numbers are dense and follow the series.

Usage:
  scripts/manage-patches.py check [PROJECT]
  scripts/manage-patches.py normalize [PROJECT]
  scripts/manage-patches.py renumber [PROJECT]
  scripts/manage-patches.py path PROJECT SLUG
  scripts/manage-patches.py export PROJECT REVISION OUTPUT [--replace]
  scripts/manage-patches.py export-worktree PROJECT OUTPUT --subject TEXT [--unified N]

check fails when a .patch file is missing from its series, when the series names a missing file, when a file is
listed twice, when the numbers are not dense, or when a patch lacks the Gnoblin author header. renumber rewrites
the directory and file numbers from the series. export and export-worktree append a new patch to the series.

For imported patches, keep the original author's credit in Original-Author.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

import patch_series
from patch_series import PATCH_ROOT, PROJECTS, ROOT

AUTHOR = "kierandrewett <kieran@drewett.dev>"
AUTHOR_LINE = re.compile(r"(?m)^From: [^\r\n]+$")


def paths(project: str | None) -> list[Path]:
    """The patch files to inspect: the listed ones of the selected projects, plus packaged patches for a full run."""
    selected = []
    for name in [project] if project else PROJECTS:
        selected += patch_series.series(name)
    if project is None:
        selected += sorted((ROOT / "packaging").rglob("*.patch"))
    return selected


def renumber(project: str) -> int:
    """Rename the files of one project to the layout its series implies, then rewrite the series."""
    lines = patch_series.entries(project)
    base = PATCH_ROOT / project
    missing = [line for line in lines if not (base / line).is_file()]
    if missing or len(set(lines)) != len(lines):
        raise patch_series.SeriesError(
            f"{project}: renumber needs every series entry to name one existing file, "
            f"found {len(missing)} missing and {len(lines) - len(set(lines))} repeated; "
            "edit the series with the current file names, then renumber"
        )
    wanted = patch_series.canonical(lines)
    stage = base / ".renumber"
    stage.mkdir()
    try:
        for number, line in enumerate(lines):
            os.rename(base / line, stage / f"{number:05d}.patch")
        for number, line in enumerate(wanted):
            (base / line).parent.mkdir(exist_ok=True)
            os.rename(stage / f"{number:05d}.patch", base / line)
    finally:
        stage.rmdir()
    for directory in sorted(base.iterdir()):
        if directory.is_dir() and not any(directory.iterdir()):
            directory.rmdir()
    patch_series.series_path(project).write_text("".join(f"{line}\n" for line in wanted))
    moved = sum(1 for old, new in zip(lines, wanted) if old != new)
    print(f"{project}: renamed {moved} of {len(lines)} patches")
    return moved


def list_new_patch(project: str, output: Path) -> None:
    """Append an exported patch to the series when it is not listed yet."""
    relative = output.relative_to(PATCH_ROOT / project).as_posix()
    lines = patch_series.entries(project)
    if relative not in lines:
        patch_series.series_path(project).write_text("".join(f"{line}\n" for line in [*lines, relative]))
        print(f"appended {relative} to {patch_series.series_path(project).relative_to(ROOT)}", file=sys.stderr)


def normalize(content: str, source: str) -> str:
    matches = list(AUTHOR_LINE.finditer(content))
    if len(matches) != 1:
        raise ValueError(f"{source}: expected exactly one From: header, found {len(matches)}")
    return AUTHOR_LINE.sub(f"From: {AUTHOR}", content, count=1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("check", "normalize", "renumber"):
        subcommand = commands.add_parser(command)
        subcommand.add_argument("project", choices=PROJECTS, nargs="?")
    where = commands.add_parser("path", help="print the one patch file whose name is <NNNN>-SLUG.patch")
    where.add_argument("project", choices=PROJECTS)
    where.add_argument("slug")
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
    working.add_argument("--unified", type=int, default=3)
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
        if not patch_series.ENTRY.fullmatch(output.relative_to(expected_root).as_posix()):
            parser.error(f"output must be {expected_root}/<NN>-<topic>/<NNNN>-<slug>.patch")
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
                    f"--unified={args.unified}",
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
        list_new_patch(args.project, output)
        print(output.relative_to(ROOT))
        return 0

    if args.command == "path":
        try:
            print(patch_series.find(args.project, args.slug))
        except patch_series.SeriesError as error:
            print(error, file=sys.stderr)
            return 1
        return 0

    if args.command == "renumber":
        try:
            for name in [args.project] if args.project else PROJECTS:
                renumber(name)
        except patch_series.SeriesError as error:
            print(error, file=sys.stderr)
            return 1
        return 0

    if args.command == "check":
        listing = [
            line for name in ([args.project] if args.project else PROJECTS) for line in patch_series.problems(name)
        ]
        if listing:
            print("\n".join(listing), file=sys.stderr)
            return 1
    try:
        selected = paths(args.project)
    except patch_series.SeriesError as error:
        print(error, file=sys.stderr)
        return 1
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
