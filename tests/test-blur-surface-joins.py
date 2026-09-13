#!/usr/bin/env python3
"""Compare one panel with adjacent layer surfaces over the same backdrop."""

import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
assert str(root).startswith("/tmp/gnoblin-gs."), "Run in the private compositor harness"
root.mkdir(parents=True, exist_ok=True)
(root / "init.lua").write_text("""return {
 shell = {['layer-animation'] = 'none'},
 ['window-rules'] = {{match = {layer = '^blur-joins$'}, blur = 24}},
}""")
qml = root / "joins.qml"
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Io
ShellRoot {
 id: root
 property bool split: false
 IpcHandler { target: "test"; function split(value: bool): void { root.split = value } }
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  color: "#333333"
  Repeater { model: 80
   Rectangle { required property int index
    x: index * 16; width: 8; height: 800
    color: index % 3 ? "#dddddd" : "#808080"
   }
  }
 }
 Variants {
  model: [{x: 0, w: 320, split: false}, {x: 0, w: 20, split: true},
          {x: 20, w: 140, split: true}, {x: 160, w: 160, split: true}]
  PanelWindow {
   required property var modelData
   visible: root.split === modelData.split
   anchors { top: true; left: true }
   margins.left: 200 + modelData.x
   margins.top: 200
   implicitWidth: modelData.w; implicitHeight: 160
   exclusionMode: ExclusionMode.Ignore
   WlrLayershell.layer: WlrLayer.Top
   WlrLayershell.namespace: "blur-joins"
   color: "#80303030"
  }
 }
}
""")
if os.environ.get("GNOBLIN_TEST_STANDARD_BLUR") == "1":
    qml.write_text(
        qml.read_text()
        .replace("import QtQuick", "import QtQuick\nimport Bingux.Effects 1.0 as Native", 1)
        .replace("required property var modelData", "id: panel\n   required property var modelData", 1)
        .replace(
            'color: "#80303030"', 'color: "#80303030"\n   Native.BackgroundEffect { target: panel.contentItem }', 1
        )
    )
qs = os.environ.get("QS_TEST_BIN", "qs")
output = Path("/tmp/gnoblin-blur-joins")
output.mkdir(exist_ok=True)
with (output / "runtime.log").open("w") as log:
    process = subprocess.Popen([qs, "-p", str(qml)], stdout=log, stderr=log)
    try:
        time.sleep(2)
        assert process.poll() is None
        subprocess.run(["grim", str(output / "single.png")], check=True)
        subprocess.run([qs, "ipc", "-p", str(qml), "call", "test", "split", "true"], check=True)
        time.sleep(1)
        subprocess.run(["grim", str(output / "split.png")], check=True)
        delta = ImageChops.difference(
            Image.open(output / "single.png").convert("RGB"), Image.open(output / "split.png").convert("RGB")
        )
        delta.save(output / "difference.png")
        for x in (220, 360):
            stats = ImageStat.Stat(delta.crop((x - 8, 220, x + 8, 340)))
            peak = max(hi for lo, hi in stats.extrema)
            print(f"join x={x}: peak={peak}, mean={max(stats.mean):.3f}", flush=True)
            assert peak <= 5, "Surface boundary changes the blur"
    finally:
        process.terminate()
        process.wait(timeout=5)
