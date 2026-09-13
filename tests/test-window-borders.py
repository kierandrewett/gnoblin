#!/usr/bin/env python3
"""Check both border rings in an isolated compositor with real window pixels."""

import json
import os
from pathlib import Path
import subprocess
import time
from PIL import Image

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
(root / "scripts").mkdir(parents=True, exist_ok=True)
config = root / "corner-test.json"


def configure(value):
    temp = config.with_suffix(".tmp")
    temp.write_text(json.dumps(value))
    temp.replace(config)
    time.sleep(0.4)


configure({})
source = (Path(__file__).resolve().parents[1] / "tests/window-corners-native.js").read_text()
source = (
    source.replace("corners?.effect", "borders?.widget")
    .replace("_action, ...corners}", "_action, _shadow, _animation, ...borders}")
    .replace("},corners}", "},borders}")
)
source = source.replace(
    "match:{type:'window'},borders",
    "match:{type:'window'},borders,corners:{radius:borders.radius ?? 0,smoothing:borders.smoothing ?? 0,mode:'force','shadow-animation':_animation ?? {duration:0},shadow:_shadow ?? (borders.radius ? {blur:24,spread:0,opacity:0.35} : false)}",
)
(root / "scripts/corner-test.js").write_text(source)
subprocess.run(
    [
        "gdbus",
        "call",
        "--session",
        "--dest",
        "org.gnoblin.Shell",
        "--object-path",
        "/org/gnoblin/Shell",
        "--method",
        "org.gnoblin.Shell.Reload",
    ],
    check=True,
)
qml = root / "border.qml"
margin = int(os.environ.get("GNOBLIN_TEST_CSD_MARGIN", "0"))
qml.write_text(
    """import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors {top:true;bottom:true;left:true;right:true}
  color: "#205080"
  WlrLayershell.layer: WlrLayer.Background
 }
 FloatingWindow {
  title: "Corner fixture"
  implicitWidth: 320; implicitHeight: 240
  color: "transparent"
  Rectangle { anchors.fill: parent; anchors.margins: CSD_MARGIN; color: "white" }
 }
}
""".replace("CSD_MARGIN", str(margin))
)
proc = subprocess.Popen(
    ["qs", "-p", str(qml)],
    stdout=subprocess.DEVNULL,
    stderr=(root / "border-client.log").open("w"),
    env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
)


def capture():
    path = root / "border.png"
    subprocess.run(["grim", str(path)], check=True)
    return Image.open(path).convert("RGB")


try:
    for _ in range(30):
        time.sleep(0.1)
        frames = root / "corner-frames.json"
        if frames.exists() and json.loads(frames.read_text()):
            break
    time.sleep(0.6)
    configure({})
    f = json.loads(frames.read_text())[0]["frame"]
    x, y, w, h = (f[k] for k in ["x", "y", "width", "height"])
    x += margin
    y += margin
    w -= margin * 2
    h -= margin * 2
    before = capture()
    rule = {"inner-width": 2, "inner-color": "#505050ff", "outer-width": 2, "outer-color": "#00000080", "radius": 14}
    configure(rule)
    after = capture()
    assert max(abs(v - 80) for v in after.getpixel((x + w // 2, y))) < 8, ("inner", after.getpixel((x + w // 2, y)))
    assert sum(after.getpixel((x + w // 2, y - 1))) < sum(before.getpixel((x + w // 2, y - 1))) - 30, (
        "outer ring missing"
    )
    assert sum(after.getpixel((x + 1, y + 1))) < sum(before.getpixel((x + 1, y + 1))) - 100, "client corner not clipped"
    assert after.getpixel((x + w // 2, y + h // 2)) == before.getpixel((x + w // 2, y + h // 2)), "interior changed"
    configure({**rule, "radius": 80, "smoothing": 1})
    crazier = capture()
    assert sum(abs(a - b) for a, b in zip(after.getpixel((x + 5, y + 5)), crazier.getpixel((x + 5, y + 5)))) > 100, (
        "border roundness did not update live",
        after.getpixel((x + 5, y + 5)),
        crazier.getpixel((x + 5, y + 5)),
    )
    broad = {"x": 0, "y": 10, "blur": 36, "spread": 0, "opacity": 0.22}
    contact = {"x": 0, "y": 2, "blur": 5, "spread": 0, "opacity": 0.28}
    point = (x + w // 2, y + h + 1)
    configure({**rule, "_shadow": [broad]})
    broad_pixel = sum(capture().getpixel(point))
    configure({**rule, "_shadow": [contact]})
    contact_pixel = sum(capture().getpixel(point))
    configure({**rule, "_shadow": [broad, contact]})
    layered_pixel = sum(capture().getpixel(point))
    assert layered_pixel < min(broad_pixel, contact_pixel) - 4, (
        "layers not combined",
        broad_pixel,
        contact_pixel,
        layered_pixel,
    )
    configure({**rule, "_shadow": [{**broad, "opacity": 0}]})
    clear_pixel = sum(capture().getpixel(point))
    configure({**rule, "_shadow": [{**broad, "opacity": 0.8}], "_animation": {"duration": 1200, "easing": "linear"}})
    middle_pixel = sum(capture().getpixel(point))
    time.sleep(1.0)
    dark_pixel = sum(capture().getpixel(point))
    assert clear_pixel > middle_pixel + 4 and middle_pixel > dark_pixel + 4, (
        "shadow did not fade",
        clear_pixel,
        middle_pixel,
        dark_pixel,
    )
    print("PASS: shadow fade has intermediate opacity and reaches its target")
    for action, keep in [("maximize", "keep-maximized"), ("fullscreen", "keep-fullscreen")]:
        configure({**rule, keep: False, "_action": action})
        assert not json.loads(frames.read_text())[0]["effect"], action + " opt-out"
        configure({**rule, "_action": action})
        frame = json.loads(frames.read_text())[0]["frame"]
        assert json.loads(frames.read_text())[0]["effect"], action + " default"
        edge = capture().getpixel((frame["x"] + frame["width"] // 2, frame["y"]))
        # With no panels, every maximised edge touches the physical monitor.
        expected_edge = (255, 255, 255) if action == "maximize" else (80, 80, 80)
        assert max(abs(value - expected) for value, expected in zip(edge, expected_edge)) < 18, (action, edge)
        configure({**rule, keep: True})
        assert json.loads(frames.read_text())[0]["effect"], keep
        configure({**rule, "_action": "un" + action})
    configure({})
    assert not json.loads(frames.read_text())[0]["effect"], "border removal"
    print("PASS: native inner/outer pixels, unchanged contents, live config, maximize/fullscreen and removal")
finally:
    proc.terminate()
    proc.wait(timeout=5)
