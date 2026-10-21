#!/usr/bin/env python3
"""Run the actual CSD get_pixels patch with real host Cogl bitmap ownership.

Texture producers are bounded substitutes; GPU rendering is not exercised.
Pinned renderer/transform functions are checked unchanged by the new patch.
No source checkout, compositor, display, or installed artifact is modified.
"""

import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
PATCHES = ROOT / "patches/mutter/63-csd-sampling"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def function(source, name):
    start = source.index(name + " (")
    start = source.rfind("\n", 0, source.rfind("\n", 0, start)) + 1
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end] + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true", help="apply only patches 0001/0002")
    parser.add_argument("--test", help="run one GLib test path")
    parser.add_argument("--cogl-pc", default="mutter-cogl-51")
    parser.add_argument("--sanitize", action="store_true", help="enable ASan/UBSan in the fixture")
    args = parser.parse_args()
    revision = json.loads((ROOT / "versions/gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", args.cogl_pc], text=True))
    with tempfile.TemporaryDirectory(prefix="gnoblin-csd-pixels-") as directory:
        temporary = Path(directory)
        run("git", "init", "-q", directory)
        for relative in ("src/compositor/meta-shaped-texture.c", "src/meta/meta-shaped-texture.h"):
            target = temporary / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(
                subprocess.check_output(
                    ["git", "-C", str(ROOT / "subprojects/mutter"), "show", f"{revision}:{relative}"]
                )
            )
        patches = sorted(PATCHES.glob("*.patch"))
        for patch in patches[:2]:
            run("git", "-C", directory, "apply", str(patch))
        target = temporary / "src/compositor/meta-shaped-texture.c"
        baseline = target.read_text()
        if not args.baseline:
            for patch in patches[2:]:
                run("git", "-C", directory, "apply", str(patch))
        source = target.read_text()
        for name in (
            "meta_shaped_texture_render_to_bitmap",
            "meta_shaped_texture_paint_to_bitmap",
            "meta_shaped_texture_should_get_via_offscreen",
        ):
            assert function(source, name) == function(baseline, name), name
        (temporary / "csd-get-pixels.inc").write_text(function(source, "meta_shaped_texture_get_pixels"))
        executable = temporary / "csd-pixel-lifetime"
        run(
            os.environ.get("CC", "cc"),
            "-std=gnu11",
            "-Wall",
            "-Wextra",
            "-Werror",
            *(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"] if args.sanitize else []),
            f"-I{temporary}",
            str(ROOT / "tests/csd-pixel-lifetime.c"),
            "-o",
            str(executable),
            *flags,
        )
        command = [str(executable)]
        if args.test:
            command += ["-p", args.test]
        run(*command)


if __name__ == "__main__":
    main()
