#!/usr/bin/env python3
"""Pixel regression for masked layer blur in an isolated Gnoblin session."""

import math
import os
import re
import shutil
import subprocess
import time
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
test_corners = os.environ.get("GNOBLIN_BLUR_TEST_CORNERS") == "1"
test_shadows = os.environ.get("GNOBLIN_BLUR_TEST_SHADOWS") == "1"
use_theme = os.environ.get("GNOBLIN_BLUR_TEST_THEME") == "1"
test_scale = float(os.environ.get("GNOBLIN_BLUR_TEST_SCALE", "1"))
if test_scale < 1:
    raise ValueError("GNOBLIN_BLUR_TEST_SCALE must be positive")
if test_scale > 1:
    prefix = Path(os.environ["GNOBLIN_PREFIX"])
    subprocess.run(
        [
            str(prefix / "bin/gdctl"),
            "set",
            "-L",
            "-M",
            "Meta-0",
            "-p",
            "-s",
            f"{test_scale:g}",
        ],
        check=True,
    )
    display = subprocess.run([str(prefix / "bin/gdctl"), "show"], check=True, capture_output=True, text=True).stdout
    scales = [float(value) for value in re.findall(r"Scale: ([0-9.]+)", display)]
    if not scales or any(not math.isclose(value, test_scale, abs_tol=0.001) for value in scales):
        raise AssertionError(f"gdctl reports scales {scales}, expected {test_scale}")
if use_theme:
    shutil.copy2(Path(__file__).resolve().parents[2] / "bingux/shell/bingux/Theme.qml", root / "Theme.qml")
    (root / "qmldir").write_text("singleton Theme 1.0 Theme.qml\n")
qml = root / "effect.qml"
qml.write_text(
    """import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  WlrLayershell.namespace: "effect-pattern"
  color: "black"
  Grid {
   columns: 160
   Repeater { model: 160 * 100
    Rectangle { required property int index; width: 8; height: 8; color: (index % 160 + Math.floor(index / 160)) % 2 ? "#eeeeee" : "#222222" }
   }
  }
 }
 PanelWindow {
  anchors { top: true; left: true }
  implicitWidth: 320; implicitHeight: 220
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "effect-mask"
  color: "transparent"
  // SHADOW_FIXTURE
  Rectangle { x: 64; y: 64; width: 192; height: 96; radius: 16; color: "#80303030" }
 }
}
""".replace(
        "implicitWidth: 320; implicitHeight: 220",
        "implicitWidth: 1280; implicitHeight: 800" if test_corners else "implicitWidth: 320; implicitHeight: 220",
    )
    .replace('color: "#80303030"', "color: Theme.popupSurface" if use_theme else 'color: "#80303030"')
    .replace(
        "// SHADOW_FIXTURE",
        'Rectangle { x: 48; y: 48; width: 224; height: 128; color: "#40000000" }\n  Rectangle { x: 276; y: 64; width: 40; height: 96; color: "#20303030" }'
        if test_shadows
        else "",
    )
)
config = root / "init.lua"


def configure(blur, opacity=1):
    ignore_shadows = (
        ', ["blur-ignore-shadows"] = true'
        if test_shadows and os.environ.get("GNOBLIN_BLUR_SHADOW_BASELINE") != "1"
        else ""
    )
    config.write_text(f"""local g = require("gnoblin")
g.set({{
    shell = {{["layer-animation"] = "none"}},
    ["window-rules"] = {{{{match = {{layer = "^effect-mask$"}}, blur = {blur}, opacity = {opacity}{ignore_shadows}}}}},
}})
""")
    time.sleep(0.5)


def capture(name):
    path = root / name
    subprocess.run(["grim", str(path)], check=True)
    image = Image.open(path)
    if test_scale > 1:
        monitor_width, monitor_height = (int(value) for value in os.environ["MONITOR"].split("x", 1))
        expected_size = (monitor_width, monitor_height)
        if image.size != expected_size:
            raise AssertionError(f"scale-{test_scale} screencopy is {image.size}, expected {expected_size}")
        logical_size = (round(image.width / test_scale), round(image.height / test_scale))
        print(f"scale-{test_scale} screencopy: {image.width}x{image.height}; checking logical pixels")
        image = image.resize(logical_size, Image.Resampling.NEAREST)
    return image.convert("RGB")


configure(0)
proc = subprocess.Popen(["qs", "-p", str(qml)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(1.5)
    before = capture("effect-before.png")
    configure(24)
    after = capture("effect-after.png")
    outside = ImageChops.difference(before.crop((8, 8, 48, 48)), after.crop((8, 8, 48, 48)))
    assert max(ImageStat.Stat(outside).mean) < 1, "transparent surface area changed"
    sharp = ImageStat.Stat(before.crop((88, 88, 232, 136))).stddev[0]
    blurred = ImageStat.Stat(after.crop((88, 88, 232, 136))).stddev[0]
    assert sharp > 1, "panel tint is opaque: no backdrop can show through"
    assert blurred < sharp * 0.10, (sharp, blurred)
    if test_corners:
        # Outside the rounded silhouette, the wallpaper must remain unchanged.
        delta = ImageChops.difference(before, after)
        edge_pixels = [
            (x, y)
            for y in range(60, 80)
            for x in range(60, 80)
            if ((x + 0.5 - 80) ** 2 + (y + 0.5 - 80) ** 2) ** 0.5 > 17
        ]
        peak_point = max(edge_pixels, key=lambda point: max(delta.getpixel(point)))
        peak = max(delta.getpixel(peak_point))
        assert peak <= 2, ("blur outside rounded corner", peak, peak_point)
        if not use_theme and not test_shadows:
            for y in range(64, 80):
                for x in range(64, 80):
                    background = 238 if (x // 8 + y // 8) % 2 else 34
                    alpha = (before.getpixel((x, y))[0] - background) / (48 - background)
                    if 0.03 < alpha < 0.4:
                        change = delta.getpixel((x, y))[0]
                        assert change <= alpha / (128 / 255) * 110 + 4, ("edge coverage expanded", x, y, alpha, change)
        print("PASS: rounded corner preserves exterior wallpaper and partial coverage")
    if test_shadows:
        delta = ImageStat.Stat(ImageChops.difference(before, after).crop((49, 88, 60, 136)))
        assert max(delta.mean) < 1.5, ("shadow pixels blurred", delta.mean)
        assert ImageStat.Stat(after.crop((282, 88, 310, 136))).stddev[0] < 2, "coloured translucent content lost blur"
        print("PASS: shadow pixels preserved; translucent coloured material still blurs")
    # Rule opacity must not weaken backdrop coverage as the client fades.
    for opacity in (0.5, 0.2):
        configure(0, opacity)
        translucent_before = capture(f"effect-opacity-{opacity}-before.png")
        configure(24, opacity)
        translucent_after = capture(f"effect-opacity-{opacity}-after.png")
        region = (88, 88, 232, 136)
        sharp_opacity = ImageStat.Stat(translucent_before.crop(region)).stddev[0]
        blurred_opacity = ImageStat.Stat(translucent_after.crop(region)).stddev[0]
        assert blurred_opacity < sharp_opacity * 0.10, (opacity, sharp_opacity, blurred_opacity)
        outside = ImageChops.difference(translucent_before.crop((8, 8, 48, 48)), translucent_after.crop((8, 8, 48, 48)))
        assert max(ImageStat.Stat(outside).mean) < 1, "opacity compensation changed transparent pixels"
    configure(0)
    restored = capture("effect-restored.png")
    assert max(ImageStat.Stat(ImageChops.difference(before, restored)).mean) < 1, "rule removal did not restore pixels"
    assert proc.poll() is None
    print(
        f"PASS: blur reduced checker contrast {sharp:.1f} -> {blurred:.1f}; transparent pixels and rule removal preserved"
    )
finally:
    proc.terminate()
    proc.wait(timeout=5)
