#!/usr/bin/env python3
"""Audit generated GLES 2.0 blur uniforms and real Cogl array uploads.

Requires a configured/built Mutter tree, C compiler and GLES/EGL development
packages. Reads that build's flags/headers, copies its Cogl/MTK libraries, and
writes only --artifacts (which must not exist). No compositor or session is
contacted. See blur-kernel-capacity.md for the minimum-capacity argument.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compile_flags(build):
    database = json.loads((build / "compile_commands.json").read_text())
    entry = next(item for item in database if item["file"].endswith("/clutter-blur.c"))
    args = entry.get("arguments") or shlex.split(entry["command"])
    if Path(args[0]).name in {"sccache", "ccache"}:
        args = args[1:]
    result = []
    i = 0
    while i < len(args):
        arg = args[i]
        if arg in {"-o", "-MF", "-MQ", "-MT"}:
            i += 2
            continue
        if arg not in {"-c", "-MD", "-MMD", entry["file"]}:
            # Some Meson databases retain literal single quotes around -D.
            result.append(arg.strip("'") if arg.startswith("'-D") else arg)
        i += 1
    return Path(entry["directory"]), result


def run_probe(source, label, root, directory, flags, libraries):
    destination = root / label
    destination.mkdir()
    executable = destination / "probe"
    command = [
        *flags,
        "-Werror",
        f'-DBLUR_SOURCE="{source}"',
        str(Path(__file__).with_name("blur-kernel-cogl.c")),
        "-o",
        str(executable),
        *map(str, libraries),
        "-Wl,-rpath," + str(root / "lib"),
        *shlex.split(
            subprocess.check_output(
                ["pkg-config", "--libs", "glib-2.0", "gobject-2.0", "gio-2.0", "egl", "glesv2"], text=True
            )
        ),
        "-lm",
    ]
    (destination / "compile.json").write_text(
        json.dumps({"directory": str(directory), "arguments": command}, indent=2) + "\n"
    )
    with (destination / "compile.log").open("w") as log:
        subprocess.run(command, cwd=directory, stdout=log, stderr=subprocess.STDOUT, check=True)
    env = dict(
        os.environ,
        COGL_DRIVER="gles2",
        MESA_GLES_VERSION_OVERRIDE="2.0",
        LIBGL_ALWAYS_SOFTWARE="1",
        LP_NUM_THREADS="2",
        LD_LIBRARY_PATH=str(root / "lib"),
    )
    with (destination / "run.log").open("w") as log:
        subprocess.run([str(executable), str(destination)], env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    lines = (destination / "run.log").read_text().splitlines()
    values = dict(
        line.split(" ", 1)
        for line in lines
        if line.startswith(("GL_VERSION ", "GLSL_VERSION ", "RENDERER ", "DRIVER_LIMIT ", "FRAGMENT_ROWS "))
    )
    assert values["GL_VERSION"].startswith("OpenGL ES 2.0"), values
    assert int(values["FRAGMENT_ROWS"]) <= 16, values
    values["uniforms"] = [line.split()[1:] for line in lines if line.startswith("FRAGMENT_UNIFORM ")]
    values["cases"] = [line.split()[1:] for line in lines if line.startswith("CASE ")]
    assert len(values["cases"]) == 33, values
    assert len({case[-1] for case in values["cases"]}) == 1, "expected a shared Cogl GL program"
    values["source_sha256"] = digest(source)
    return values


def compare(root):
    results = []
    for candidate in sorted((root / "candidate").glob("*.rgba")):
        reference = root / "reference" / candidate.name
        actual, expected = candidate.read_bytes(), reference.read_bytes()
        assert len(actual) == len(expected) > 0
        maximum = max(abs(a - b) for a, b in zip(actual, expected, strict=True))
        assert maximum <= 1, (candidate.name, maximum)
        for directory in ("reference", "candidate"):
            first = root / directory / candidate.name.replace("round-1", "round-0").replace("round-2", "round-0")
            assert (root / directory / candidate.name).read_bytes() == first.read_bytes(), (
                directory,
                candidate.name,
                "state leaked between instances",
            )
        results.append({"file": candidate.name, "max_rgba8_difference": maximum})
    assert len(results) == 33
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mutter-build", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--reference-source", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    root = args.artifacts.resolve()
    root.mkdir(parents=True, exist_ok=False)
    (root / "lib").mkdir()
    build = args.mutter_build.resolve()
    libraries = []
    for component in ("cogl", "mtk"):
        matches = list((build / component / component).glob(f"libmutter-{component}-*.so.0"))
        assert len(matches) == 1, matches
        target = root / "lib" / matches[0].name
        shutil.copyfile(matches[0], target)
        libraries.append(target)
    directory, flags = compile_flags(build)
    report = {"libraries": {path.name: digest(path) for path in libraries}}
    for label, source in (("reference", args.reference_source), ("candidate", args.source)):
        report[label] = run_probe(source.resolve(), label, root, directory, flags, libraries)
    report["pixel_comparisons"] = compare(root)
    report["capacity_basis"] = (
        "Conservative active fragment rows <= GLES 2.0 minimum 16; driver hardware limit is reported separately."
    )
    (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "reference_rows": report["reference"]["FRAGMENT_ROWS"],
                "candidate_rows": report["candidate"]["FRAGMENT_ROWS"],
                "pixel_cases": len(report["pixel_comparisons"]),
                "report": str(root / "report.json"),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
