#!/usr/bin/env python3
"""Pixel regression for masked layer blur in an isolated Gnoblin session."""
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
root.mkdir(parents=True, exist_ok=True)
qml = root / 'effect.qml'
qml.write_text('''import QtQuick
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
  Rectangle { x: 64; y: 64; width: 192; height: 96; radius: 16; color: "#80303030" }
 }
}
''')
config = root / 'gnoblin.toml'
def configure(blur):
    config.write_text(f'[shell]\nlayer-animation="none"\n[[window-rules]]\nmatch.layer="^effect-mask$"\nblur={blur}\n')
    time.sleep(.5)
def capture(name):
    path = root / name
    subprocess.run(['grim', str(path)], check=True)
    return Image.open(path).convert('RGB')
configure(0)
proc = subprocess.Popen(['qs', '-p', str(qml)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(1.5)
    before = capture('effect-before.png')
    configure(24)
    after = capture('effect-after.png')
    outside = ImageChops.difference(before.crop((8, 8, 48, 48)), after.crop((8, 8, 48, 48)))
    assert max(ImageStat.Stat(outside).mean) < 1, 'transparent surface area changed'
    sharp = ImageStat.Stat(before.crop((88, 88, 232, 136))).stddev[0]
    blurred = ImageStat.Stat(after.crop((88, 88, 232, 136))).stddev[0]
    assert blurred < sharp * .85, (sharp, blurred)
    configure(0)
    restored = capture('effect-restored.png')
    assert max(ImageStat.Stat(ImageChops.difference(before, restored)).mean) < 1, 'rule removal did not restore pixels'
    assert proc.poll() is None
    print(f'PASS: blur reduced checker contrast {sharp:.1f} -> {blurred:.1f}; transparent pixels and rule removal preserved')
finally:
    proc.terminate()
    proc.wait(timeout=5)
