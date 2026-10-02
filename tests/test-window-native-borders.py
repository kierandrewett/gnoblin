#!/usr/bin/env python3
"""Verify Lua window rules draw borders in a standalone Gnoblin session."""

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
red = (255, 0, 0)

config.write_text(
    "gnoblin.window_rule {\n"
    '    match = {title = "^Native border fixture$"},\n'
    '    corners = {radius = 12, mode = "force", border_width = 6, border_color = "#ff0000"},\n'
    "}\n"
)
subprocess.run([gnoblinctl, "config", "reload"], check=True)

qml = root / "native-border-client.qml"
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
        title: "Native border fixture"
        implicitWidth: 320; implicitHeight: 240
        color: "transparent"
        Rectangle { anchors.fill: parent; color: "white"; radius: 40 }
    }
}
"""
)

with (root / "native-border-client.log").open("w") as log:
    process = subprocess.Popen(
        ["qs", "-p", str(qml)],
        stdout=log,
        stderr=log,
        env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    )
    try:
        geometry = None
        for _ in range(100):
            result = subprocess.run(
                [gnoblinctl, "--json", "window", "list", "--title", "Native border fixture"],
                check=True,
                capture_output=True,
                text=True,
            )
            windows = json.loads(result.stdout)["windows"]
            geometry = windows[0]["geometry"] if windows else None
            if geometry:
                break
            time.sleep(0.1)
        assert geometry, "native border fixture did not appear in the window list"
        box = (
            geometry["x"],
            geometry["y"],
            geometry["x"] + geometry["width"],
            geometry["y"] + geometry["height"],
        )
        screenshot = root / "native-border-screen.png"
        image = None
        for _ in range(50):
            subprocess.run(["grim", str(screenshot)], check=True)
            image = Image.open(screenshot).convert("RGB").crop(box)
            if image.getpixel((160, 2)) == red:
                break
            time.sleep(0.1)

        assert image.getpixel((160, 2)) == red, (
            "native border did not color the configured top edge",
            image.getpixel((160, 2)),
        )
        assert image.getpixel((160, 12)) == (255, 255, 255), (
            "native border changed the window body",
            image.getpixel((160, 12)),
        )
        assert image.getpixel((1, 1)) == background, (
            "native border painted outside the rounded window",
            image.getpixel((1, 1)),
        )

        config.write_text(
            "gnoblin.window_rule {\n"
            '    match = {title = "^Native border fixture$"},\n'
            '    corners = {radius = 0, mode = "force", border_width = 6, border_color = "#ff0000"},\n'
            "}\n"
        )
        subprocess.run([gnoblinctl, "config", "reload"], check=True)
        for _ in range(50):
            subprocess.run(["grim", str(screenshot)], check=True)
            image = Image.open(screenshot).convert("RGB").crop(box)
            if image.getpixel((1, 1)) == red:
                break
            time.sleep(0.1)
        assert image.getpixel((1, 1)) == red, (
            "radius-zero border did not produce a rectangular outline",
            image.getpixel((1, 1)),
        )
        print("PASS: Lua border rules paint the native window edge without changing its body")
    finally:
        process.terminate()
        process.wait(timeout=5)
