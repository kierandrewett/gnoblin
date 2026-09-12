#!/usr/bin/env python3
"""Pixel checks for shaders, blur composition, reload and rejected shader edits."""

import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
shader = root / "test.frag"
config = root / "init.lua"
qml = root / "shader.qml"
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  WlrLayershell.namespace: "shader-background"
  color: "#aaaaaa"
 }
 PanelWindow {
  anchors { top: true; left: true }
  implicitWidth: 320; implicitHeight: 220
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "shader-card"
  color: "transparent"
  Rectangle { x: 64; y: 64; width: 192; height: 96; radius: 16; color: "#80303030" }
 }
}
""")


def write_shader(colour):
    temporary = shader.with_suffix(".tmp")
    temporary.write_text(
        "uniform float strength;\nvec4 gnoblin_effect(vec4 color, vec2 uv) {"
        f" return vec4(mix(color.rgb, vec3({colour}), strength), color.a); }}"
    )
    temporary.replace(shader)
    time.sleep(0.5)


def configure(enabled=True, strength=1, blur=0, path="test.frag"):
    shader = f', shader = {path!r}, ["shader-uniforms"] = {{strength = {strength}.0}}' if enabled else ""
    config.write_text(f"""local g = require("gnoblin")
g.set({{
    shell = {{["layer-animation"] = "none"}},
    ["window-rules"] = {{{{match = {{layer = "^shader-card$"}}, blur = {blur}{shader}}}}},
}})
""")
    time.sleep(0.5)


def capture():
    path = root / "frame.png"
    subprocess.run(["grim", str(path)], check=True)
    return Image.open(path).convert("RGB")


def same(a, b):
    return max(ImageStat.Stat(ImageChops.difference(a, b)).mean) < 1


configure(False)
proc = subprocess.Popen(["qs", "-p", str(qml)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(1)
    original = capture()
    write_shader("1.0, 0.0, 0.0")
    configure()
    red = capture()
    pixel = red.getpixel((128, 100))
    assert pixel[0] > pixel[1] + 80 and pixel[0] > pixel[2] + 80, ("red shader missing", pixel)
    assert same(original.crop((0, 0, 40, 40)), red.crop((0, 0, 40, 40))), "transparent surface filled"
    configure(blur=24)
    red = capture()
    pixel = red.getpixel((128, 100))
    assert pixel[0] > pixel[1] + 80, ("shader lost when blur was added", pixel)
    assert same(original.crop((0, 0, 40, 40)), red.crop((0, 0, 40, 40))), "blur/shader filled transparent area"
    write_shader("0.0, 0.0, 1.0")
    blue = capture()
    pixel = blue.getpixel((128, 100))
    assert pixel[2] > pixel[0] + 80, ("shader did not hot reload", pixel)
    shader.write_text("this is not GLSL")
    time.sleep(0.5)
    assert same(blue, capture()), "invalid shader replaced the previous effect"
    shader.unlink()
    time.sleep(0.5)
    assert same(blue, capture()), "deleted file replaced the previous effect"
    write_shader("1.0, 0.0, 0.0")
    assert same(red, capture()), "shader did not recover after file recreation"
    configure(path="missing.frag", blur=24)
    assert same(red, capture()), "missing new path removed the previous effect"
    configure(strength=0)
    assert same(original, capture()), "config uniforms did not update"
    configure(False)
    assert same(original, capture()), "removing the shader rule did not restore pixels"
    assert proc.poll() is None
    print(
        "PASS: shader pixels, alpha, blur composition, atomic reload, invalid/missing file recovery, uniforms and rule removal"
    )
finally:
    proc.terminate()
    proc.wait(timeout=5)
