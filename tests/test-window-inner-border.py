#!/usr/bin/env python3
"""Verify the corners inner line and outline paint one pixel each, on both sides of the window edge."""

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
title = "Inner border fixture"
backdrop = (0x20, 0x50, 0x80)
body = (0x33, 0x66, 0x99)
red = (0xFF, 0x00, 0x00)
green = (0x00, 0xFF, 0x00)


def configure(corners):
    config.write_text(
        "gnoblin.window_rule {\n"
        f'    match = {{title = "^{title}$"}},\n'
        f"    corners = {{radius = 12, shadow = false, {corners}}},\n"
        "}\n"
    )
    result = subprocess.run([gnoblinctl, "config", "reload"], capture_output=True, text=True)
    assert result.returncode == 0, f"config reload failed: {result.stdout}{result.stderr}"


configure(
    'mode = "force", border_width = -1, border_color = "#ff0000", '
    'inner_border_width = 1, inner_border_color = "#00ff00"'
)

qml = root / "inner-border-client.qml"
qml.write_text(
    f"""import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {{
    PanelWindow {{
        anchors {{ top: true; bottom: true; left: true; right: true }}
        WlrLayershell.layer: WlrLayer.Background
        color: "#205080"
    }}
    FloatingWindow {{
        title: "{title}"
        implicitWidth: 320; implicitHeight: 240
        color: "#336699"
    }}
}}
"""
)


def window_frame():
    for _ in range(100):
        result = subprocess.run(
            [gnoblinctl, "--json", "window", "list", "--title", title],
            check=True,
            capture_output=True,
            text=True,
        )
        windows = json.loads(result.stdout)["windows"]
        if windows:
            return windows[0]["frame"], windows[0]["monitor_id"]
        time.sleep(0.1)
    raise AssertionError("inner border fixture did not appear in the window list")


def edge_pixels(frame, monitor, screenshot):
    """Return (outside, first inside, second inside) for the left, top, right and bottom edges."""
    subprocess.run(["grim", "-o", monitor, str(screenshot)], check=True)
    image = Image.open(screenshot).convert("RGB")
    left, top = frame["x"], frame["y"]
    right, bottom = left + frame["width"], top + frame["height"]
    middle_x, middle_y = (left + right) // 2, (top + bottom) // 2
    return {
        "left": [
            image.getpixel((left - 1, middle_y)),
            image.getpixel((left, middle_y)),
            image.getpixel((left + 1, middle_y)),
        ],
        "right": [
            image.getpixel((right, middle_y)),
            image.getpixel((right - 1, middle_y)),
            image.getpixel((right - 2, middle_y)),
        ],
        "top": [
            image.getpixel((middle_x, top - 1)),
            image.getpixel((middle_x, top)),
            image.getpixel((middle_x, top + 1)),
        ],
        "bottom": [
            image.getpixel((middle_x, bottom)),
            image.getpixel((middle_x, bottom - 1)),
            image.getpixel((middle_x, bottom - 2)),
        ],
    }, image


def wait_for(screenshot, expected):
    """Poll until every edge shows `expected`; the window may still move while it maps."""
    pixels = None
    for _ in range(100):
        frame, monitor = window_frame()
        pixels, image = edge_pixels(frame, monitor, screenshot)
        if all(side == expected for side in pixels.values()):
            return image, frame
        time.sleep(0.1)
    raise AssertionError(f"edge pixels {pixels} did not become {expected}")


with (root / "inner-border-client.log").open("w") as log:
    process = subprocess.Popen(
        ["qs", "-p", str(qml)],
        stdout=log,
        stderr=log,
        env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    )
    try:
        window_frame()
        screenshot = root / "inner-border-screen.png"

        # Outline one pixel outside the edge, inner line in the window's own first pixel.
        image, frame = wait_for(screenshot, [red, green, body])
        # The corner is rounded, so the first window pixel at the very corner is not painted.
        assert image.getpixel((frame["x"], frame["y"])) == backdrop, (
            "inner line painted the clipped window corner",
            image.getpixel((frame["x"], frame["y"])),
        )

        # Removing the inner line leaves the outline and the body untouched.
        configure('mode = "force", border_width = -1, border_color = "#ff0000"')
        wait_for(screenshot, [red, body, body])

        # The inner line alone needs no outline.
        configure('mode = "force", inner_border_width = 1, inner_border_color = "#00ff00"')
        wait_for(screenshot, [backdrop, green, body])

        # Mode off disables both lines.
        configure(
            'mode = "off", border_width = -1, border_color = "#ff0000", '
            'inner_border_width = 1, inner_border_color = "#00ff00"'
        )
        wait_for(screenshot, [backdrop, body, body])
        print("PASS: the inner line and outline paint on either side of the window edge")
    finally:
        process.terminate()
        process.wait(timeout=5)
