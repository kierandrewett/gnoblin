#!/usr/bin/env python3
"""Verify Lua replacement shadows in a standalone Gnoblin session."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from PIL import Image, ImageChops, ImageStat

assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
config = root / "init.lua"
gnoblinctl = os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or "gnoblinctl"
title = "Native shadow fixture"


def configure(shadow):
    corners = 'radius = 24, mode = "force"'
    if shadow:
        corners += ', shadow = {x = 0, y = 4, blur = 20, spread = 0, opacity = 0.8, color = "#000000"}'
    config.write_text(
        f'gnoblin.window_rule {{\n    match = {{title = "^{title}$"}},\n    corners = {{{corners}}},\n}}\n'
    )
    subprocess.run([gnoblinctl, "config", "reload"], check=True)


def capture():
    path = root / "native-shadow-screen.png"
    subprocess.run(["grim", str(path)], check=True)
    return Image.open(path).convert("RGB")


configure(False)
qml = root / "native-shadow-client.qml"
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
        title: "Native shadow fixture"
        implicitWidth: 320; implicitHeight: 240
        color: "transparent"
        Rectangle { anchors.fill: parent; color: "#80ffffff"; radius: 40 }
    }
}
"""
)

with (root / "native-shadow-client.log").open("w") as log:
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
                [gnoblinctl, "--json", "window", "list", "--title", title],
                check=True,
                capture_output=True,
                text=True,
            )
            windows = json.loads(result.stdout)["windows"]
            geometry = windows[0]["geometry"] if windows else None
            if geometry:
                break
            time.sleep(0.1)
        assert geometry, "native shadow fixture did not appear in the window list"

        box = (
            geometry["x"],
            geometry["y"],
            geometry["x"] + geometry["width"],
            geometry["y"] + geometry["height"],
        )
        baseline = capture()
        shadow_region = (
            box[2] + 2,
            box[1] + 90,
            box[2] + 16,
            box[1] + 150,
        )
        body_point = (box[0] + 160, box[1] + 120)
        rounded_corner = (box[0] + 2, box[1] + 2)

        configure(True)
        shadowed = None
        for _ in range(50):
            time.sleep(0.1)
            shadowed = capture()
            difference = ImageChops.difference(baseline.crop(shadow_region), shadowed.crop(shadow_region))
            if sum(ImageStat.Stat(difference).sum) > 100:
                break

        difference = ImageChops.difference(baseline.crop(shadow_region), shadowed.crop(shadow_region))
        assert sum(ImageStat.Stat(difference).sum) > 100, (
            "Lua replacement shadow did not change pixels outside the window",
            ImageStat.Stat(difference).sum,
        )
        assert shadowed.getpixel(body_point) == baseline.getpixel(body_point), (
            "replacement shadow changed translucent window contents",
            baseline.getpixel(body_point),
            shadowed.getpixel(body_point),
        )
        assert shadowed.getpixel(rounded_corner) == baseline.getpixel(rounded_corner), (
            "replacement shadow painted through the rounded window cutout",
            baseline.getpixel(rounded_corner),
            shadowed.getpixel(rounded_corner),
        )

        configure(False)
        restored = None
        for _ in range(50):
            time.sleep(0.1)
            restored = capture()
            difference = ImageChops.difference(baseline.crop(shadow_region), restored.crop(shadow_region))
            if sum(ImageStat.Stat(difference).sum) < 20:
                break
        difference = ImageChops.difference(baseline.crop(shadow_region), restored.crop(shadow_region))
        assert sum(ImageStat.Stat(difference).sum) < 20, (
            "removing the shadow rule did not restore the original background",
            ImageStat.Stat(difference).sum,
        )
        print("PASS: standalone Lua shadow rules render, preserve window pixels, and clear on reload")
    finally:
        process.terminate()
        process.wait(timeout=5)
