#!/usr/bin/env python3
"""Verify a Lua corner rule clips client shadows in the running session."""

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


def configure(mode):
    temporary = config.with_suffix(".tmp")
    temporary.write_text(
        f"""gnoblin.window_rule {{
    match = {{title = "^Corner fixture$"}},
    corners = {{radius = 14, padding = {{20, 20, 20, 20}}, mode = {json.dumps(mode)}, shadow = false}},
}}
"""
    )
    temporary.replace(config)
    subprocess.run([gnoblinctl, "config", "reload"], check=True)


def frame():
    result = subprocess.run(
        [gnoblinctl, "--json", "window", "list", "--title", "Corner fixture"],
        check=True,
        capture_output=True,
        text=True,
    )
    windows = json.loads(result.stdout)["windows"]
    if windows:
        return windows[0]["geometry"]
    return None


configure("auto")
qml = root / "shadow-client.qml"
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
    PanelWindow {
        anchors { top: true; bottom: true; left: true; right: true }
        WlrLayershell.layer: WlrLayer.Background
        color: "#205080"
    }
    FloatingWindow {
        title: "Corner fixture"
        implicitWidth: 320; implicitHeight: 240
        color: "transparent"
        Rectangle { anchors.fill: parent; color: "#80000000" }
        Rectangle { anchors.fill: parent; anchors.margins: 20; color: "white" }
    }
}
""")
with (root / "shadow-client.log").open("w") as log:
    process = subprocess.Popen(
        ["qs", "-p", str(qml)], stdout=log, stderr=log, env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"}
    )
    try:
        f = None
        for _ in range(50):
            f = frame()
            if f:
                break
            time.sleep(0.1)
        assert f, "Corner fixture did not appear in the native window snapshot"

        def pixels():
            image = root / "shadow-screen.png"
            subprocess.run(["grim", str(image)], check=True)
            return Image.open(image).convert("RGB")

        before = pixels()
        outside = (f["x"] + 10, f["y"] + f["height"] // 2)
        assert sum(before.getpixel(outside)) < 230, "fixture has no client shadow"
        configure("force")
        after = pixels()
        assert after.getpixel(outside) == (32, 80, 128), (
            "forced clipping retained client shadow",
            after.getpixel(outside),
        )
        assert after.getpixel((f["x"] + 21, f["y"] + 21)) == (32, 80, 128), "corner not clipped"
        assert after.getpixel((f["x"] + 160, f["y"] + 120)) == (255, 255, 255), "body changed"
        configure("auto")
        assert pixels().getpixel(outside) == before.getpixel(outside), "automatic shadow policy changed"
        print("PASS: forced corners clear client shadows without replacement; auto preserves them")
    finally:
        process.terminate()
        process.wait(timeout=5)
