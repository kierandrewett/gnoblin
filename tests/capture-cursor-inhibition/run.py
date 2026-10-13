#!/usr/bin/env python3
"""Compile pinned Mutter's cursor-inhibitor and area/monitor lifecycle functions."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import re
import runpy
import shlex
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PATCH = ROOT / "patches/mutter/66-capture-cursor/0001-refresh-cursors-on-stream-inhibition.patch"
EXTRACT = runpy.run_path(str(ROOT / "tests/native-window-publish.py"))["function"]
BACKEND = "src/backends/meta-backend.c"
FUNCTIONS = {
    BACKEND: [
        "static gboolean\nupdate_cursor_foreach_cb (",
        "static void\nupdate_cursors (",
        "gboolean\nmeta_backend_is_hw_cursors_inhibited (",
        "void\nmeta_backend_add_hw_cursor_inhibitor (",
        "void\nmeta_backend_remove_hw_cursor_inhibitor (",
    ],
    **{
        f"src/backends/meta-stream-source-{kind}.c": [
            "static void\ninhibit_hw_cursor (",
            "static void\nuninhibit_hw_cursor (",
            f"static void\nmeta_stream_source_{kind}_enable (",
            f"static void\nmeta_stream_source_{kind}_disable (",
        ]
        for kind in ("area", "monitor")
    },
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true", help="expect embedded cases to fail")
    parser.add_argument("--source-tree", type=Path, help="read an already prepared tree")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--test", help="one GLib test path")
    args = parser.parse_args()
    if args.baseline and args.source_tree:
        parser.error("--baseline and --source-tree are mutually exclusive")
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    revision = json.loads((ROOT / "gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    with tempfile.TemporaryDirectory(prefix="gnoblin-capture-cursor-") as scratch:
        directory = Path(scratch)
        tree = args.source_tree.resolve() if args.source_tree else directory
        if not args.source_tree:
            subprocess.run(["git", "init", "--quiet", scratch], check=True)
            for name in FUNCTIONS:
                path = tree / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(
                    subprocess.check_output(
                        ["git", "-C", str(ROOT / "subprojects/mutter"), "show", f"{revision}:{name}"]
                    )
                )
            if not args.baseline:
                subprocess.run(["git", "apply", "--check", "--whitespace=error", str(PATCH)], cwd=tree, check=True)
                subprocess.run(["git", "apply", str(PATCH)], cwd=tree, check=True)
        for name, signatures in FUNCTIONS.items():
            source = (tree / name).read_text()
            pieces = []
            for signature in signatures:
                body = EXTRACT(source, signature)
                line = source[: source.index(body)].count("\n") + 1
                pieces.append(f'#line {line} "{tree / name}"\n{body}\n')
            output = "backend" if name == BACKEND else Path(name).stem.removeprefix("meta-stream-source-")
            (directory / f"{output}.inc").write_text("\n".join(pieces))
            print(f"{name}: {hashlib.sha256(source.encode()).hexdigest()}", flush=True)
        flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "glib-2.0"], text=True))
        binary = directory / "regression"
        command = shlex.split(os.environ.get("CC", "cc")) + [
            "-std=gnu11",
            "-g",
            "-O1",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            "-Wno-unused-function",
            "-I",
            scratch,
            str(HERE / "fixture.c"),
            "-o",
            str(binary),
            *flags,
        ]
        if args.sanitize:
            command += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
        subprocess.run(command, check=True)
        env = {**os.environ, "G_DEBUG": "fatal-criticals"}
        if args.sanitize:
            env.update(ASAN_OPTIONS="detect_leaks=1:abort_on_error=1", UBSAN_OPTIONS="halt_on_error=1")
        listing = subprocess.check_output([str(binary), "-l"], text=True)
        tests = re.findall(r"^(?:# )?(/\S+)$", listing, re.MULTILINE)
        if not tests:
            raise RuntimeError(f"No GLib tests found: {listing}")
        if args.test:
            if args.test not in tests:
                parser.error(f"unknown test: {args.test}")
            tests = [args.test]
        failures = []
        for test in tests:
            result = subprocess.run([str(binary), "-p", test], capture_output=True, text=True, timeout=5, env=env)
            print(f"{'FAIL' if result.returncode else 'PASS'} {test}", flush=True)
            if result.returncode:
                print(result.stdout + result.stderr, end="", flush=True)
                failures.append(test)
        print(f"{len(failures)} failed", flush=True)
        return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
