#!/usr/bin/env python3
"""Exercise Mutter's production filter-to-actor boundary without a desktop."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shlex
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PATCH = ROOT / "patches/mutter/99-typed-window-api/0125-route-diagnostic-input-before-wayland.patch"
FUNCTIONS = {
    "clutter/clutter/clutter-main.c": ["void\nclutter_stage_handle_event ("],
    "clutter/clutter/clutter-event.c": ["gboolean\n_clutter_event_process_filters ("],
    "src/core/events.c": ["static gboolean\nmeta_display_handle_event (", "static gboolean\nevent_callback ("],
    "src/wayland/meta-wayland-pointer.c": ["gboolean\nmeta_wayland_pointer_handle_event ("],
}


def syntax_check(directory, commands):
    wanted = {"events.c": directory / "src/core/events.c"}
    wanted.update(
        {name: ROOT / "src/compositor/plugin" / name for name in ("gnoblin-plugin.c", "gnoblin-plugin-ui.cpp")}
    )
    checked = set()
    for entry in json.loads(commands.read_text()):
        name = Path(entry["file"]).name
        if name not in wanted or (name == "events.c" and "/core/" not in entry["file"]):
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
                command.append(str(wanted[name]) if arg == entry["file"] else arg)
            index += 1
        command[1:1] = ["-fsyntax-only", f"-I{directory / 'src'}"]
        subprocess.run(command, cwd=entry["directory"], check=True)
        print("syntax checked:", wanted[name].name, flush=True)
        checked.add(name)
    if checked != set(wanted):
        raise RuntimeError(f"compile database is missing: {set(wanted) - checked}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mutter-repo", type=Path, default=ROOT / "subprojects/mutter")
    parser.add_argument("--imgui-source", type=Path, default=ROOT / "subprojects/imgui")
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--test")
    parser.add_argument("--compile-commands", type=Path, help="read-only full translation-unit syntax checks")
    args = parser.parse_args()
    extract = runpy.run_path(str(ROOT / "tests/native-window-publish.py"))["function"]
    pin = json.loads((ROOT / "gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    with tempfile.TemporaryDirectory(prefix="gnoblin-recovery-input-") as temporary:
        directory = Path(temporary)
        subprocess.run(["git", "init", "-q", str(directory)], check=True)
        for name in FUNCTIONS:
            target = directory / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(subprocess.check_output(["git", "-C", str(args.mutter_repo), "show", f"{pin}:{name}"]))
        for patch in sorted((ROOT / "patches/mutter").rglob("*.patch")):
            if args.baseline and patch == PATCH:
                continue
            subprocess.run(
                ["git", "-C", str(directory), "apply", *[f"--include={name}" for name in FUNCTIONS], str(patch)],
                check=True,
                capture_output=True,
            )
        # Keep production branches and ordering verbatim. Stubs implement only
        # device/service boundaries; they do not decide the route under test.
        source = "\n\n".join(
            extract((directory / name).read_text(), signature)
            for name, signatures in reversed(FUNCTIONS.items())
            for signature in signatures
        )
        (directory / "routing.inc").write_text(source)
        print("routing SHA256:", hashlib.sha256(source.encode()).hexdigest(), flush=True)
        patched = "gnoblin_plugin_input_handle_event" in source
        plugin = ROOT / "src/compositor/plugin"
        if args.compile_commands:
            bridge = directory / "src/compositor/plugins/gnoblin-plugin-input.h"
            bridge.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(plugin / bridge.name, bridge)
            syntax_check(directory, args.compile_commands)
        (directory / "ui-input.inc").write_text(
            "\n\n".join(
                [
                    extract(
                        (plugin / "gnoblin-imgui-overlay.cpp").read_text(), "void gnoblin_imgui_overlay_set_pointer("
                    ),
                    extract((plugin / "gnoblin-plugin-ui.cpp").read_text(), "static gboolean console_take_key("),
                    extract((plugin / "gnoblin-plugin-ui.cpp").read_text(), "gboolean gnoblin_plugin_ui_handle_event("),
                ]
            )
        )
        (directory / "clutter").mkdir(exist_ok=True)
        (directory / "clutter/clutter.h").write_text('#include "events.h"\n')
        flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "gobject-2.0"], text=True))
        common = [
            "-g",
            "-O1",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            f"-I{directory}",
            f"-I{Path(__file__).parent}",
            f"-I{plugin}",
            *[flag for flag in flags if not flag.startswith("-l")],
        ]
        if args.sanitize:
            common += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
        command = shlex.split(os.environ.get("CC", "cc")) + [
            "-std=gnu11",
            "-g",
            "-O1",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            "-Wno-unused-function",
            "-Wno-unused-variable",
            f"-DPATCHED={int(patched)}",
            f"-I{directory}",
            "-c",
            str(Path(__file__).with_name("regression.c")),
            "-o",
            str(directory / "routing.o"),
            *common,
        ]
        subprocess.run(command, check=True)
        subprocess.run(
            shlex.split(os.environ.get("CXX", "c++"))
            + [
                "-std=c++17",
                *common,
                f"-I{args.imgui_source}",
                str(Path(__file__).with_name("panel.cpp")),
                str(plugin / "gnoblin-recovery-panel.cpp"),
                *[
                    str(args.imgui_source / name)
                    for name in ("imgui.cpp", "imgui_draw.cpp", "imgui_tables.cpp", "imgui_widgets.cpp")
                ],
                str(directory / "routing.o"),
                "-o",
                str(directory / "regression"),
                *flags,
            ],
            check=True,
        )
        subprocess.run([str(directory / "regression"), *(["-p", args.test] if args.test else [])], check=True)


if __name__ == "__main__":
    main()
