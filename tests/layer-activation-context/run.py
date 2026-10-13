#!/usr/bin/env python3
"""Compile the production activation path from pinned source and patch series."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PATCH = ROOT / "patches/mutter/99-typed-window-api/0124-preserve-layer-activation-context.patch"
FILES = [
    f"src/wayland/meta-wayland-{name}"
    for name in ("activation.c", "activation.h", "keyboard.c", "pointer.c", "seat.c", "touch.c", "tablet-tool.c")
]


def prepare(directory, baseline):
    pin = json.loads((ROOT / "gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    subprocess.run(["git", "init", "-q", str(directory)], check=True)
    for name in FILES:
        target = directory / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(
            subprocess.check_output(["git", "-C", str(ROOT / "subprojects/mutter"), "show", f"{pin}:{name}"])
        )
    for patch in sorted((ROOT / "patches/mutter").rglob("*.patch")):
        if baseline and patch == PATCH:
            continue
        subprocess.run(
            ["git", "-C", str(directory), "apply", *[f"--include={name}" for name in FILES], str(patch)],
            check=True,
            capture_output=True,
        )
    print(f"pinned Mutter: {pin}", flush=True)


def syntax_check(source_tree, commands):
    wanted = {Path(name).name for name in FILES if name.endswith(".c")}
    checked = set()
    for entry in json.loads(commands.read_text()):
        name = Path(entry["file"]).name
        if name not in wanted:
            continue
        args = entry.get("arguments") or shlex.split(entry["command"])
        if Path(args[0]).name in ("sccache", "ccache"):
            args = args[1:]
        command = []
        index = 0
        while index < len(args):
            arg = args[index]
            if arg in ("-o", "-MF", "-MQ", "-MT"):
                index += 2
                continue
            if arg not in ("-MD", "-MMD", "-c"):
                command.append(str(source_tree / "src/wayland" / name) if arg == entry["file"] else arg)
            index += 1
        command[1:1] = ["-fsyntax-only", "-I" + str(source_tree / "src")]
        subprocess.run(command, cwd=entry["directory"], check=True)
        checked.add(name)
        print("syntax checked:", name, flush=True)
    if checked != wanted:
        raise RuntimeError(f"compile database is missing: {wanted - checked}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--source-tree", type=Path, help="read a prepared disposable tree")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--compile-commands", type=Path, help="also syntax-check complete changed C files")
    parser.add_argument("--test", help="one GLib test path")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="gnoblin-layer-activation-") as temporary:
        directory = Path(temporary)
        source_tree = args.source_tree
        if source_tree is None:
            source_tree = directory
            prepare(directory, args.baseline)
        if args.compile_commands:
            syntax_check(source_tree, args.compile_commands)
        source = (source_tree / FILES[0]).read_text()
        # Compile the entire production translation unit. Only includes are
        # replaced by boundary types below; no policy function is reimplemented.
        source = re.sub(r"^#include .*$", "", source, flags=re.MULTILINE)
        (directory / "activation.inc").write_text(source)
        print("activation SHA256:", hashlib.sha256(source.encode()).hexdigest(), flush=True)
        extract = runpy.run_path(str(ROOT / "tests/native-window-publish.py"))["function"]
        keyboard = (source_tree / FILES[2]).read_text()
        pointer = (source_tree / FILES[3]).read_text()
        delivery = [
            extract(pointer, "static gboolean\npointer_can_grab_surface ("),
            extract(pointer, "static gboolean\nmeta_wayland_pointer_can_grab_surface ("),
            extract(pointer, "gboolean\nmeta_wayland_pointer_get_grab_info ("),
            extract(keyboard, "gboolean\nmeta_wayland_keyboard_can_popup ("),
            extract(keyboard, "gboolean\nmeta_wayland_keyboard_can_grab_surface ("),
            extract(keyboard, "static gboolean\nmeta_wayland_keyboard_broadcast_key ("),
            extract(pointer, "static void\nmeta_wayland_pointer_send_button ("),
        ]
        delivery += [
            extract((source_tree / FILES[5]).read_text(), "static void\nhandle_touch_begin ("),
            extract((source_tree / FILES[6]).read_text(), "static void\nbroadcast_down ("),
            extract((source_tree / FILES[6]).read_text(), "static void\nbroadcast_button ("),
        ]
        (directory / "input-delivery.inc").write_text("\n\n".join(delivery))
        patched = "gnoblin_layer_source" in source
        if patched:
            seat = (source_tree / FILES[4]).read_text()
            assert "meta_wayland_activation_update_input (seat->compositor, event);" in seat
            assert "meta_wayland_activation_record_input (" in keyboard
            assert "meta_wayland_activation_record_input (" in pointer
        flags = shlex.split(
            subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0", "wayland-server"], text=True)
        )
        protocol = Path("/usr/share/wayland-protocols/staging/xdg-activation/xdg-activation-v1.xml")
        for mode, target in [
            ("server-header", "xdg-activation-v1-server-protocol.h"),
            ("private-code", "xdg-activation-v1-protocol.c"),
        ]:
            subprocess.run(["wayland-scanner", mode, str(protocol), str(directory / target)], check=True)
        tablet_protocol = Path("/usr/share/wayland-protocols/stable/tablet/tablet-v2.xml")
        for mode, target in [
            ("server-header", "tablet-v2-server-protocol.h"),
            ("private-code", "tablet-v2-protocol.c"),
        ]:
            subprocess.run(["wayland-scanner", mode, str(tablet_protocol), str(directory / target)], check=True)
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
            f"-DPATCHED={int(patched)}",
            f"-I{directory}",
            str(Path(__file__).with_name("regression.c")),
            str(directory / "xdg-activation-v1-protocol.c"),
            str(directory / "tablet-v2-protocol.c"),
            "-o",
            str(binary),
            *flags,
        ]
        if args.sanitize:
            command += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
        subprocess.run(command, check=True)
        subprocess.run([str(binary), *(["-p", args.test] if args.test else [])], check=True)


if __name__ == "__main__":
    main()
