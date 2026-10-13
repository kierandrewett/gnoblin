#!/usr/bin/env python3
"""Run the real window lifecycle against JSON-GLib snapshots without a desktop.

python3 tests/native-window-focus-snapshot.py
CC=clang python3 tests/native-window-focus-snapshot.py --sanitize
Add --build-dir DIR to retain the executable; --monitors-source FILE accepts a
saved baseline. Arguments after -- are passed to GLib (e.g. -- -p /focus/transfer).
Only socket/Lua transport boundaries are stubbed. This does not test native I/O.
"""

import argparse
import hashlib
import os
from pathlib import Path
import runpy
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONTROL = ROOT / "src/control"
# Reuse the existing verbatim-function reader without editing the publisher test.
extract = runpy.run_path(str(ROOT / "tests/native-window-publish.py"))["function"]


def build(directory, monitors, sanitize):
    directory.mkdir(parents=True, exist_ok=True)
    pieces = []
    sources = {
        name: path.read_text()
        for name, path in {
            "publish": CONTROL / "gnoblin-control-publish.c",
            "wire": CONTROL / "gnoblin-control-wire.c",
            "state": CONTROL / "gnoblin-native-control.c",
            "monitors": monitors,
        }.items()
    }
    publish = sources["publish"]
    start = publish.index("const WindowPropertyName window_property_names[]")
    end = publish.index("static void native_window_record_add_public_aliases(", start)
    pieces.append(publish[start:end])
    for name, signatures in (
        ("wire", ["JsonNode* json_from_variant(", "GVariant* variant_from_json("]),
        ("publish", ["gboolean window_record_is_modal(", "JsonNode* lua_window_record("]),
        ("state", ["void native_window_state_free("]),
        (
            "monitors",
            [
                "static gboolean window_property_equal(",
                "static JsonArray* changed_window_properties(",
                "static void dispatch_lua_window_event(",
                "void publish_window_changes(",
            ],
        ),
    ):
        pieces.extend(extract(sources[name], signature) for signature in signatures)
    (directory / "lifecycle.inc").write_text("\n\n".join(pieces))
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0", "json-glib-1.0"], text=True)
    )
    binary = directory / "native-window-focus-snapshot"
    command = shlex.split(os.environ.get("CC", "cc")) + [
        "-std=c11",
        "-O2",
        "-g",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-I",
        str(directory),
        str(ROOT / "tests/native-window-focus-snapshot-test.c"),
        "-o",
        str(binary),
        *flags,
    ]
    if sanitize:
        command += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
    print(shlex.join(command), flush=True)
    subprocess.run(command, check=True)
    for name, source in sources.items():
        print(name, hashlib.sha256(source.encode()).hexdigest(), flush=True)
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--monitors-source", type=Path, default=CONTROL / "gnoblin-control-monitors.c")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("test_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    test_args = args.test_args[1:] if args.test_args[:1] == ["--"] else args.test_args
    with tempfile.TemporaryDirectory(prefix="gnoblin-focus-snapshot-") as temporary:
        binary = build(args.build_dir or Path(temporary), args.monitors_source, args.sanitize)
        subprocess.run([str(binary), *test_args], check=True)


if __name__ == "__main__":
    main()
