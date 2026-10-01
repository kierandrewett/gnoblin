#!/usr/bin/env python3
"""Verify Lua window rules replace transparent client-drawn corners natively."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from PIL import Image

assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
config = root / "init.lua"
gnoblinctl = os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or "gnoblinctl"
background = (32, 80, 128)


def configure(remove_csd):
    corners = 'radius = 12, mode = "auto"'
    if remove_csd:
        corners += ", remove_csd = true"
    temporary = config.with_suffix(".tmp")
    temporary.write_text(
        "gnoblin.window_rule {\n"
        '    match = {title = "^CSD reconstruction fixture$"},\n'
        f"    corners = {{{corners}}},\n"
        "}\n"
    )
    temporary.replace(config)
    subprocess.run([gnoblinctl, "config", "reload"], check=True)


def frame():
    result = subprocess.run(
        [gnoblinctl, "--json", "window", "list", "--title", "CSD reconstruction fixture"],
        check=True,
        capture_output=True,
        text=True,
    )
    windows = json.loads(result.stdout)["windows"]
    return windows[0]["geometry"] if windows else None


def capture(box=None):
    path = root / "csd-reconstruction-screen.png"
    subprocess.run(["grim", str(path)], check=True)
    image = Image.open(path).convert("RGB")
    return image.crop(box) if box else image


configure(False)
qml = root / "csd-reconstruction-client.qml"
qml.write_text(
    """import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
    PanelWindow {
        anchors { top: true; bottom: true; left: true; right: true }
        WlrLayershell.layer: WlrLayer.Background
        color: "#205080"
    }
    FloatingWindow {
        title: "CSD reconstruction fixture"
        implicitWidth: 320; implicitHeight: 240
        color: "transparent"
        Rectangle { anchors.fill: parent; color: "white"; radius: 40 }
    }
}
"""
)
with (root / "csd-reconstruction-client.log").open("w") as log:
    process = subprocess.Popen(
        ["qs", "-p", str(qml)],
        stdout=log,
        stderr=log,
        env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    )
    try:
        geometry = None
        for _ in range(100):
            geometry = frame()
            if geometry:
                break
            time.sleep(0.1)
        assert geometry, "CSD reconstruction fixture did not appear in the window list"
        box = (
            geometry["x"],
            geometry["y"],
            geometry["x"] + geometry["width"],
            geometry["y"] + geometry["height"],
        )
        original = capture(box)
        assert original.getpixel((8, 8)) == background, (
            "fixture did not render a transparent rounded corner",
            original.getpixel((8, 8)),
        )

        configure(True)
        restored = None
        for _ in range(100):
            restored = capture(box)
            if restored.getpixel((8, 8)) == (255, 255, 255):
                break
            time.sleep(0.1)

        assert restored.getpixel((8, 8)) == (255, 255, 255), (
            "native CSD reconstruction did not fill the transparent corner",
            restored.getpixel((8, 8)),
        )
        assert restored.getpixel((1, 1)) == background, "configured compositor corner was not clipped"
        assert restored.getpixel((160, 120)) == (255, 255, 255), "reconstruction changed the window body"
        print("PASS: Lua remove_csd restores client corner pixels before native rounding")
    finally:
        process.terminate()
        process.wait(timeout=5)
