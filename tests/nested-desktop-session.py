#!/usr/bin/env python3
"""Keep a normal-config nested desktop alive for input/pixel regression checks.

Run only through run-gnome-devkit.sh, with a copied XDG_CONFIG_HOME and private
XDG_RUNTIME_DIR. Does not install fixture rules or replace the user's settings.
"""

import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-devkit-")
root = Path(os.environ["XDG_CONFIG_HOME"]).parent
assert str(root).startswith("/tmp/gnoblin-user-config.")
env = {
    key: value
    for key, value in os.environ.items()
    if key
    in (
        "DBUS_SESSION_BUS_ADDRESS",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "PATH",
        "LD_LIBRARY_PATH",
        "GI_TYPELIB_PATH",
        "BINGUX_CONFIG_PATH",
        "GNOBLIN_COMPOSITOR_SOCKET",
        "GDK_BACKEND",
        "QT_QPA_PLATFORM",
        "BINGUX_QUICKSHELL",
    )
}
(root / "session.json").write_text(json.dumps(env))
processes = []
try:
    for name, command in (
        (
            "bingux",
            [
                os.environ.get("BINGUX_QUICKSHELL", "gnoblin-quickshell"),
                "-p",
                os.environ["BINGUX_CONFIG_PATH"],
                "--no-color",
            ],
        ),
        ("gtk", ["gjs", "-m", str(Path(__file__).with_name("window-corners-gtk.js"))]),
        ("ghostty", ["ghostty", "--title=Nested Ghostty", "-e", "bash", "--noprofile", "--norc"]),
    ):
        log = (root / f"{name}.log").open("w")
        processes.append(subprocess.Popen(command, stdout=log, stderr=log))
        log.close()
    print("NESTED NORMAL-CONFIG SESSION:", root, flush=True)
    while not (root / "stop").exists():
        time.sleep(0.25)
finally:
    for process in processes:
        if process.poll() is None:
            process.terminate()
    for process in processes:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
