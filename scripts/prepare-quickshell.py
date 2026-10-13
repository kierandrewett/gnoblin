#!/usr/bin/env python3
"""Apply Gnoblin's desktop-action fix before an external Quickshell build.

Accept only the exact parser from a1a150fab00a93ea983aaca5df55304bc837f51b,
or that parser with our patch already applied. Other files are untouched.
This does not fetch, reset, configure, build, install, or change a Git index.
"""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parent.parent
TARGET = Path("src/core/desktopentry.cpp")
PATCH = ROOT / "patches/quickshell/0001-preserve-desktop-action-identifiers.patch"
BEFORE = "58e7e5807ed8566dee1006725d58a842737d754d560da454e0fcff2589223f68"
AFTER = "9a2b668d770e4a0e40841ba898ac08aae4a6af1f5b88d6d223dd2f5e84eb18c3"


def digest(content):
    return hashlib.sha256(content).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="external pinned Quickshell source directory")
    parser.add_argument("--check", action="store_true", help="report readiness without modifying source")
    args = parser.parse_args()
    source = args.source.resolve()
    target = source / TARGET
    if target.is_symlink() or not target.resolve().is_relative_to(source) or not target.is_file():
        parser.error(f"expected a regular source file inside {source}: {TARGET}")
    original = target.read_bytes()
    current = digest(original)
    if current == AFTER:
        print("Quickshell desktop-action patch already applied.")
        return 0
    if current != BEFORE:
        parser.error(f"unrecognized {TARGET} ({current}); inspect the source before rebasing this patch")
    if args.check:
        print("Quickshell pinned parser recognized; desktop-action patch is required before building.")
        return 1
    if not shutil.which("patch"):
        parser.error("GNU patch is required")
    with tempfile.TemporaryDirectory(prefix="gnoblin-prepare-quickshell-") as temporary:
        staged = Path(temporary) / TARGET
        staged.parent.mkdir(parents=True)
        staged.write_bytes(original)
        subprocess.run(
            ["patch", "--batch", "--forward", "--fuzz=0", "-p1", "-i", str(PATCH)],
            cwd=temporary,
            check=True,
        )
        updated = staged.read_bytes()
    if digest(updated) != AFTER:
        parser.error("patch output differs from the reviewed parser; source was not changed")
    if target.read_bytes() != original:
        parser.error("parser changed during preparation; source was not changed")
    with tempfile.NamedTemporaryFile(dir=target.parent, prefix=".desktopentry-", delete=False) as handle:
        replacement = Path(handle.name)
        try:
            handle.write(updated)
            handle.flush()
            os.fchmod(handle.fileno(), target.stat().st_mode & 0o777)
            os.replace(replacement, target)
        finally:
            replacement.unlink(missing_ok=True)
    print(f"Applied Quickshell desktop-action patch: {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
