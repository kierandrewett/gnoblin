#!/usr/bin/env python3
"""Native pixel checks. Run as GNOBLIN_TEST_DBUS_CLIENT in an isolated shell."""

import json
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
config = root / "corner-test.json"


def configure(value):
    temporary = config.with_suffix(".tmp")
    temporary.write_text(json.dumps(value))
    temporary.replace(config)
    time.sleep(0.4)


configure({"radius": 0})
(root / "scripts").mkdir(exist_ok=True)
(root / "scripts/corner-test.js").write_text(
    (Path(__file__).resolve().parents[1] / "tests/window-corners-native.js").read_text()
)
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
time.sleep(0.3)


def capture():
    path = root / "corner-screen.png"
    subprocess.run(["grim", str(path)], check=True)
    return Image.open(path).convert("RGB")


def scene(radius, opacity):
    qml = root / "corners.qml"
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
  visible: true
  implicitWidth: 320; implicitHeight: 240
  color: "transparent"
  Rectangle { anchors.fill: parent; color: Qt.rgba(1,1,1,OPACITY); radius: RADIUS }
 }
}
""".replace("RADIUS", str(radius)).replace("OPACITY", str(opacity))
    )
    proc = subprocess.Popen(
        ["qs", "-p", str(qml)],
        stdout=subprocess.DEVNULL,
        stderr=(root / "client.log").open("w"),
        env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    )
    time.sleep(0.8)
    frames = json.loads((root / "corner-frames.json").read_text())
    assert len(frames) == 1, frames
    frame = frames[0]["frame"]
    return proc, (frame["x"], frame["y"], frame["x"] + frame["width"], frame["y"] + frame["height"])


for shape, opacity in ((0, 1), (40, 1), (120, 1), (0, 0.5), (40, 0.5)):
    configure({"radius": 0})
    proc, box = scene(shape, opacity)
    try:
        baseline = capture()
        plain = baseline.crop(box)
        configure({"radius": 24, "mode": "auto"})
        rounded = capture().crop(box)
        if shape == 0:
            reference = rounded
        if shape == 0:
            assert max(plain.getpixel((2, 2))) > 180
            assert sum(rounded.getpixel((2, 2))) < sum(plain.getpixel((2, 2))) - 80, "square corner not clipped"
            assert rounded.getpixel((160, 120)) == plain.getpixel((160, 120)), "interior changed"
            configure({"radius": 24, "smoothing": 0.6, "mode": "auto"})
            smooth = capture().crop(box)
            assert sum(ImageStat.Stat(ImageChops.difference(rounded, smooth)).sum) > 100, "smoothing had no effect"
            assert sum(smooth.getpixel((20, 20))) > 500, "smoothing expanded the requested radius"
            configure({"radius": 24, "mode": "force", "border-width": 3, "border-color": "#ff0000ff"})
            border = capture().crop(box)
            assert border.getpixel((160, 1))[0] > 200 and border.getpixel((160, 1))[1] < 60, "border not rendered"
            configure({"radius": 24, "mode": "auto", "shadow": {"blur": 16, "opacity": 0.6}})
            shadow = capture()
            shadow.save("/tmp/gnoblin-corner-shadow.png")
            edge = (box[2] + 4, box[1] + 120)
            assert sum(shadow.getpixel(edge)) < sum(baseline.getpixel(edge)) - 10, (
                "custom shadow not visible",
                shadow.getpixel(edge),
                baseline.getpixel(edge),
            )
            assert shadow.getpixel((box[0] + 160, box[1] + 120)) == baseline.getpixel((box[0] + 160, box[1] + 120)), (
                "shadow painted over contents"
            )
            for action, keep in [("maximize", "keep-maximized"), ("fullscreen", "keep-fullscreen")]:
                configure({"radius": 24, keep: False, "_action": action})
                assert not json.loads((root / "corner-frames.json").read_text())[0]["effect"], (
                    action + " opt-out did not suspend rounding"
                )
                configure({"radius": 24, "_action": action})
                frame = json.loads((root / "corner-frames.json").read_text())[0]
                assert frame["effect"] == (action == "maximize"), action + " default policy"
                if action == "maximize":
                    corner = capture().getpixel((frame["frame"]["x"] + 2, frame["frame"]["y"] + 2))
                    assert sum(corner) < 500, ("maximized corner mask missing", corner)
                configure({"radius": 24, keep: True})
                assert json.loads((root / "corner-frames.json").read_text())[0]["effect"], (
                    keep + " did not restore rounding"
                )
                configure({"radius": 24, "_action": "un" + action})
        else:
            assert max(ImageStat.Stat(ImageChops.difference(plain, rounded)).mean) < 0.2, (
                "automatic mode modified an existing rounded/shaped corner"
            )
            configure({"radius": 24, "mode": "auto", "remove-csd": True})
            removed = capture().crop(box)
            assert sum(plain.getpixel((8, 8))) < 500, ("fixture has no gap", shape)
            assert sum(removed.getpixel((8, 8))) > sum(plain.getpixel((8, 8))) + 80, (
                "CSD corner was not filled",
                shape,
                removed.getpixel((8, 8)),
                json.loads((root / "corner-frames.json").read_text()),
            )
            assert sum(removed.getpixel((1, 1))) < 500, "CSD fill escaped compositor rounding"
            assert max(ImageStat.Stat(ImageChops.difference(reference, removed)).mean) < 0.5, (
                "CSD reconstruction left edge gaps"
            )
            assert removed.getpixel((160, 120)) == plain.getpixel((160, 120)), "CSD remover changed interior contents"
            configure({"radius": 24, "mode": "auto"})
            preserved = capture().crop(box)
            assert max(ImageStat.Stat(ImageChops.difference(plain, preserved)).mean) < 0.2, (
                "disabling CSD remover did not restore native corners"
            )
        configure({"radius": 0})
        restored = capture().crop(box)
        assert max(ImageStat.Stat(ImageChops.difference(plain, restored)).mean) < 0.2, (
            "removing rule changed window pixels"
        )
    finally:
        proc.terminate()
        proc.wait(timeout=5)
        time.sleep(0.2)
configure({"radius": 0})
colour_file = root / "client-colour.txt"
colour_file.write_text("#ffffff")
proc = subprocess.Popen(
    ["gjs", "-m", str(Path(__file__).resolve().parents[1] / "tests/window-corners-gtk.js")],
    stdout=subprocess.DEVNULL,
    env={**os.environ, "GNOBLIN_CSD_COLOUR_FILE": str(colour_file)},
)
try:
    for _ in range(100):
        time.sleep(0.05)
        frames = json.loads((root / "corner-frames.json").read_text())
        if frames:
            break
        assert proc.poll() is None, "GTK fixture exited before mapping"
    assert frames, "GTK fixture did not map"
    frame = frames[0]["frame"]
    box = (frame["x"], frame["y"], frame["x"] + frame["width"], frame["y"] + frame["height"])
    plain = capture().crop(box)
    configure({"radius": 4, "mode": "force", "remove-csd": True})
    filled = capture().crop(box)
    assert sum(filled.getpixel((3, 3))) > sum(plain.getpixel((3, 3))) + 80, (
        "GTK CSD gap not filled",
        plain.getpixel((3, 3)),
        filled.getpixel((3, 3)),
        json.loads((root / "corner-frames.json").read_text()),
    )
    assert filled.getpixel((160, 120)) == plain.getpixel((160, 120)), "GTK contents changed"
    # Reprobe a client that already has the reconstruction effect installed.
    # Initial-map-only checks miss cached offscreen paint feeding back into
    # corner detection during script/config reloads.
    for repeat in range(3):
        configure({"radius": 4, "mode": "force", "remove-csd": True, "_action": "reprobe"})
        reprobed = capture().crop(box)
        assert max(ImageStat.Stat(ImageChops.difference(filled, reprobed)).mean) < 0.2, (
            "GTK corner fill changed after reprobe",
            repeat,
            json.loads((root / "corner-frames.json").read_text()),
        )
    for x in range(2, 11):
        for y in range(2, 11):
            for px, py in [
                (x, y),
                (filled.width - 1 - x, y),
                (x, filled.height - 1 - y),
                (filled.width - 1 - x, filled.height - 1 - y),
            ]:
                assert min(filled.getpixel((px, py))) >= 250, (
                    "GTK native corner rim survived reconstruction",
                    px,
                    py,
                    filled.getpixel((px, py)),
                )
    # Only the client changes: no focus, resize, or compositor rule refresh.
    for colour in ["#226688", "#993366", "#333337", "#eeeeec"]:
        pending_colour = colour_file.with_suffix(".tmp")
        pending_colour.write_text(colour)
        pending_colour.replace(colour_file)
        time.sleep(0.15)
        updated = capture().crop(box)
        expected = updated.getpixel((120, 12))
        requested = tuple(int(colour[i : i + 2], 16) for i in (1, 3, 5))
        assert max(abs(a - b) for a, b in zip(expected, requested)) <= 3, (
            "client did not paint requested colour",
            colour,
            expected,
        )
        for x in range(2, 11):
            for y in range(2, 11):
                for px, py in [
                    (x, y),
                    (updated.width - 1 - x, y),
                    (x, updated.height - 1 - y),
                    (updated.width - 1 - x, updated.height - 1 - y),
                ]:
                    actual = updated.getpixel((px, py))
                    assert max(abs(a - b) for a, b in zip(actual, expected)) <= 3, (
                        "stale or outlined CSD fill",
                        colour,
                        px,
                        py,
                        actual,
                        expected,
                    )
finally:
    proc.terminate()
    proc.wait(timeout=5)
print(
    "PASS: native circular/smoothed corners, border, shadows, translucent contents, state policies, CSD reconstruction and real Libadwaita frame"
)
