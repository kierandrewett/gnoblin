#!/usr/bin/env python3
"""Pixel regression for tooltip configure/ack/buffer ordering in private Gnoblin."""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
from PIL import Image

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"), "Use the private test session"
repo = Path(__file__).resolve().parents[1]
config = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
config.mkdir(parents=True, exist_ok=True)
(config / "init.lua").write_text("""local g = require("gnoblin")
g.set({shell = {["layer-animation"] = "none"}})
""")
time.sleep(0.4)
with tempfile.TemporaryDirectory(prefix="gnoblin-layer-resize-") as directory:
    build = Path(directory)
    layer = repo / "src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
    protocols = subprocess.check_output(["pkg-config", "--variable=pkgdatadir", "wayland-protocols"], text=True).strip()
    for mode, xml, filename in [
        ("client-header", layer, "wlr-layer-shell-unstable-v1-client-protocol.h"),
        ("private-code", layer, "layer.c"),
        ("private-code", Path(protocols) / "stable/xdg-shell/xdg-shell.xml", "xdg.c"),
    ]:
        subprocess.run(["wayland-scanner", mode, str(xml), str(build / filename)], check=True)
    flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True))
    subprocess.run(
        [
            "cc",
            "-I" + str(build),
            str(repo / "tests/layer-shell-resize-client.c"),
            str(build / "layer.c"),
            str(build / "xdg.c"),
            *flags,
            "-o",
            str(build / "client"),
        ],
        check=True,
    )
    for anchor in ["bottom", "right", "top"]:
        proc = subprocess.Popen(
            [str(build / "client"), anchor], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )
        boxes = []
        try:
            for phase in ["INITIAL", "PENDING", "RESIZED"]:
                assert proc.stdout.readline().strip() == phase
                time.sleep(0.15)
                path = build / "frame.png"
                subprocess.run(["grim", str(path)], check=True)
                image = Image.open(path).convert("RGB")
                mask = Image.new("L", image.size)
                mask.putdata([255 if r > 240 and g < 10 and b > 240 else 0 for r, g, b in image.getdata()])
                boxes.append(mask.getbbox())
                proc.stdin.write("\n")
                proc.stdin.flush()
            assert boxes[0] == boxes[1], (anchor, "old buffer moved before replacement", boxes)
            assert boxes[2][2] - boxes[2][0] == 300 and boxes[2][3] - boxes[2][1] == 100, boxes
            if anchor == "bottom":
                assert boxes[0][3] == boxes[2][3], boxes
            if anchor == "right":
                assert boxes[0][2] == boxes[2][2], boxes
            if anchor == "top":
                assert boxes[2][0] == 50 and boxes[2][1] == boxes[0][1], boxes
            print(f"PASS: {anchor} tooltip keeps old pixels fixed until resized content arrives: {boxes}")
        finally:
            proc.terminate()
            proc.wait(timeout=5)
