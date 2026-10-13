#!/usr/bin/env python3
"""Compile the native window publication path without starting a compositor.

Run: python3 tests/native-window-publish.py
Keep an executable: add --build-dir /var/tmp/window-publish
Measure: add --benchmark 20000 (dispatches, not desktop frames).
Compare a saved publisher: add --publish-source /path/to/publish-before.c.
Use --compat-only on the pre-optimization source; the default also requires
zero socket encodes when no client is eligible. No installed files are used.

Complete production functions are extracted verbatim, with #line provenance.
The C harness uses real GLib/JSON-GLib; compositor snapshot providers and the
socket/Lua transport boundaries are stubs. It does not prove live socket I/O,
Lua execution, compositor scheduling, or desktop performance.
"""

import argparse
import hashlib
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONTROL = ROOT / "src/control"


def function(source, signature):
    match = re.search(re.escape(signature) + r"[^;{]*\{", source)
    if not match:
        raise ValueError(f"Missing definition: {signature}")
    depth = 1
    for end in range(match.end(), len(source)):
        depth += (source[end] == "{") - (source[end] == "}")
        if depth == 0:
            return source[match.start() : end + 1]
    raise ValueError(f"Unterminated definition: {signature}")


def build(directory, publisher, sanitize):
    directory.mkdir(parents=True, exist_ok=True)
    sources = {}

    def read(name):
        path = publisher if name == "gnoblin-control-publish.c" else CONTROL / name
        sources[path] = path.read_text()
        return path, sources[path]

    def functions(name, signatures):
        path, source = read(name)
        chunks = []
        for signature in signatures:
            body = function(source, signature)
            line = source[: source.index(body)].count("\n") + 1
            chunks.append(f'#line {line} "{path}"\n{body}\n')
        return "\n".join(chunks)

    path, publish = read("gnoblin-control-publish.c")
    start = publish.index("const WindowPropertyName window_property_names[]")
    end = publish.index("static void cache_lua_window_snapshot(", start)
    records = f'#line {publish[:start].count(chr(10)) + 1} "{path}"\n' + publish[start:end]
    wire = functions("gnoblin-control-wire.c", ["JsonNode* json_from_variant(", "GVariant* variant_from_json("])
    snapshot = functions(
        "gnoblin-control-publish.c",
        ["static void cache_lua_window_snapshot(", "JsonNode* window_snapshot_json("],
    )
    socket = functions(
        "gnoblin-control-publish.c",
        [
            "static void remove_private_focus_context(",
            "static void native_window_event_add_compat_aliases(",
            "void publish_native_socket_event(",
        ],
    )
    lifecycle = functions("gnoblin-native-control.c", ["void native_window_state_free("])
    lifecycle += functions(
        "gnoblin-control-monitors.c",
        [
            "static gboolean window_property_equal(",
            "static JsonArray* changed_window_properties(",
            "static void dispatch_lua_window_event(",
            "void publish_window_changes(",
        ],
    )
    callback = functions("gnoblin-control-signals.c", ["static gboolean publish_windows("])
    (directory / "production.inc").write_text(
        wire
        + records
        + snapshot
        + "\n#define json_to_string counted_json_to_string\n#define json_node_copy counted_json_node_copy\n"
        + socket
        + "\n#undef json_to_string\n#undef json_node_copy\n"
        + lifecycle
        + callback
    )
    binary = directory / "native-window-publish"
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0", "json-glib-1.0"], text=True)
    )
    command = shlex.split(os.environ.get("CC", "cc")) + [
        "-std=c11",
        "-O2",
        "-g",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wno-unused-parameter",
        "-I",
        str(directory),
        str(ROOT / "tests/native-window-publish-test.c"),
        "-o",
        str(binary),
        *flags,
    ]
    if sanitize:
        command += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
    print(shlex.join(command), flush=True)
    subprocess.run(command, check=True)
    for path, source in sources.items():
        print(hashlib.sha256(source.encode()).hexdigest(), path, flush=True)
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--publish-source", type=Path, default=CONTROL / "gnoblin-control-publish.c")
    parser.add_argument("--compat-only", action="store_true")
    parser.add_argument("--benchmark", type=int)
    parser.add_argument("--sanitize", action="store_true")
    args = parser.parse_args()
    if args.benchmark is not None and args.benchmark <= 0:
        parser.error("--benchmark must be positive")
    with tempfile.TemporaryDirectory(prefix="gnoblin-window-publish-") as temporary:
        binary = build(args.build_dir or Path(temporary), args.publish_source.resolve(), args.sanitize)
        command = [str(binary)]
        if args.benchmark:
            command += ["--benchmark", str(args.benchmark)]
        elif args.compat_only:
            command += ["-p", "/compat"]
        subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
