"""One place that starts external programs, so tests can replace it and every command is logged the same way."""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Mapping, Sequence


class CommandError(RuntimeError):
    pass


def quote(command: Sequence[object]) -> str:
    return " ".join(shlex.quote(str(part)) for part in command)


def run(
    command: Sequence[object],
    *,
    cwd: Path | None = None,
    env: Mapping[str, str] | None = None,
    capture: bool = False,
    stdin: str | None = None,
    quiet: bool = False,
) -> str:
    """Run a command, raise CommandError with its output on failure, and return its standard output."""
    if not quiet:
        print("+ " + quote(command), file=sys.stderr)
    merged = None if env is None else {**os.environ, **env}
    try:
        result = subprocess.run(
            [str(part) for part in command],
            cwd=cwd,
            env=merged,
            input=stdin,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            stderr=subprocess.PIPE if capture else None,
            check=False,
        )
    except FileNotFoundError as error:
        raise CommandError(f"{command[0]} is not installed ({error})") from error
    if result.returncode != 0:
        detail = (result.stderr or "").strip() if capture else ""
        raise CommandError(
            f"{quote(command)} failed with exit status {result.returncode}" + (f"\n{detail}" if detail else "")
        )
    return result.stdout or ""


def require(*programs: str) -> None:
    """Fail early and clearly when a program the next step needs is missing."""
    import shutil

    missing = [program for program in programs if shutil.which(program) is None]
    if missing:
        raise CommandError(
            "missing program"
            + ("s" if len(missing) > 1 else "")
            + ": "
            + ", ".join(missing)
            + ". Run the tool in the container image of scripts/pkgrepo/container (see its README)."
        )
