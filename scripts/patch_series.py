"""Read and validate patches/<project>/series, the single source of patch order.

Every reader of the patch trees imports this module instead of globbing and sorting. The CMake, Nix and shell
readers parse the same file. Its format is one path per line, relative to patches/<project>/ and written
<NN>-<topic>/<NNNN>-<slug>.patch, with no blank lines and no comments.
"""

from __future__ import annotations

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent
PATCH_ROOT = ROOT / "patches"
PROJECTS = ("mutter", "xdg-desktop-portal-gnome", "portal-dependencies")
SERIES = "series"
ENTRY = re.compile(
    r"(?P<dir>(?P<dirno>\d{2})-(?P<topic>[a-z0-9][a-z0-9-]*))/(?P<file>(?P<number>\d{4})-(?P<slug>[A-Za-z0-9][A-Za-z0-9._+-]*)\.patch)"
)
LAYOUT_HINT = "run scripts/manage-patches.py renumber PROJECT"


class SeriesError(ValueError):
    pass


def series_path(project: str) -> Path:
    return PATCH_ROOT / project / SERIES


def entries(project: str) -> list[str]:
    """The series lines of one project, as written. A missing or malformed file raises SeriesError."""
    path = series_path(project)
    try:
        text = path.read_text()
    except FileNotFoundError:
        raise SeriesError(f"{path.relative_to(ROOT)}: missing; every patch project needs a series file") from None
    if not text.endswith("\n"):
        raise SeriesError(f"{path.relative_to(ROOT)}: must end with a newline")
    lines = text[:-1].split("\n") if text != "\n" else []
    for number, line in enumerate(lines, 1):
        if not ENTRY.fullmatch(line):
            raise SeriesError(
                f"{path.relative_to(ROOT)}:{number}: {line!r} is not <NN>-<topic>/<NNNN>-<slug>.patch "
                "(no blank lines, comments or surrounding spaces)"
            )
    return lines


def series(project: str) -> list[Path]:
    """Absolute patch paths of one project in apply order."""
    return [PATCH_ROOT / project / line for line in entries(project)]


def find(project: str, slug: str) -> Path:
    """The one patch of a project whose file name is <NNNN>-<slug>.patch."""
    matches = [
        path
        for path in series(project)
        if ENTRY.fullmatch(path.relative_to(PATCH_ROOT / project).as_posix())["slug"] == slug
    ]
    if len(matches) != 1:
        raise SeriesError(f"{project}: {len(matches)} patches named *-{slug}.patch in the series, expected one")
    return matches[0]


def topic(project: str, name: str) -> list[Path]:
    """The patches of one topic directory (<NN>-<name>) in apply order."""
    matches = [
        path
        for path in series(project)
        if ENTRY.fullmatch(path.relative_to(PATCH_ROOT / project).as_posix())["topic"] == name
    ]
    if not matches:
        raise SeriesError(f"{project}: no patches in topic {name!r}")
    return matches


def canonical(lines: list[str]) -> list[str]:
    """The layout the series implies: topic directories numbered by first appearance, files by position in a topic."""
    topics: list[str] = []
    counts: dict[str, int] = {}
    result = []
    for line in lines:
        match = ENTRY.fullmatch(line)
        name = match["topic"]
        if name not in topics:
            topics.append(name)
        counts[name] = counts.get(name, 0) + 1
        result.append(f"{topics.index(name) + 1:02d}-{name}/{counts[name]:04d}-{match['slug']}.patch")
    return result


def problems(project: str) -> list[str]:
    """Every way the series and the patch files of one project disagree. Empty when they agree."""
    try:
        lines = entries(project)
    except SeriesError as error:
        return [str(error)]
    base = PATCH_ROOT / project
    shown = series_path(project).relative_to(ROOT)
    found = []
    seen = set()
    for line in lines:
        if line in seen:
            found.append(f"{shown}: {line} is listed twice")
        seen.add(line)
        if not (base / line).is_file():
            found.append(f"{shown}: lists {line}, which does not exist")
    for path in sorted(base.rglob("*.patch")):
        relative = path.relative_to(base).as_posix()
        if relative not in seen:
            found.append(f"{path.relative_to(ROOT)}: not listed in {shown}")
    if not found:
        expected = canonical(lines)
        for line, wanted in zip(lines, expected):
            if line != wanted:
                found.append(
                    f"{shown}: {line} should be {wanted} (numbers are dense and follow the series; {LAYOUT_HINT})"
                )
                break
    return found
