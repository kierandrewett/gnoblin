#!/usr/bin/env python3
"""Verify forced corners remove client shadow pixels without a replacement shadow."""
import json
import os
from pathlib import Path
import subprocess
import time
from PIL import Image

assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')
root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
(root / 'scripts').mkdir(parents=True, exist_ok=True)
config = root / 'corner-test.json'

def configure(mode):
    temporary = config.with_suffix('.tmp')
    temporary.write_text(json.dumps({'radius': 14, 'padding': [20, 20, 20, 20],
                                    'mode': mode, 'shadow': False}))
    temporary.replace(config)
    time.sleep(.4)

configure('auto')
(root / 'scripts/corner-test.js').write_text(
    (Path(__file__).resolve().parents[1] / 'tests/window-corners-native.js').read_text())
subprocess.run(['gnoblinctl', 'script', 'reload'], check=True)
qml = root / 'shadow-client.qml'
qml.write_text('''import QtQuick
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
''')
with (root / 'shadow-client.log').open('w') as log:
    process = subprocess.Popen(['qs', '-p', str(qml)], stdout=log, stderr=log,
        env={**os.environ, 'QT_WAYLAND_DISABLE_WINDOWDECORATION': '1'})
    try:
        frames = root / 'corner-frames.json'
        for _ in range(50):
            if frames.exists() and json.loads(frames.read_text()):
                break
            time.sleep(.1)
        f = json.loads(frames.read_text())[0]['frame']
        def pixels():
            image = root / 'shadow-screen.png'
            subprocess.run(['grim', str(image)], check=True)
            return Image.open(image).convert('RGB')
        before = pixels()
        outside = (f['x'] + 10, f['y'] + f['height'] // 2)
        assert sum(before.getpixel(outside)) < 230, 'fixture has no client shadow'
        configure('force')
        after = pixels()
        assert after.getpixel(outside) == (32, 80, 128), (
            'forced clipping retained client shadow', after.getpixel(outside))
        assert after.getpixel((f['x'] + 21, f['y'] + 21)) == (32, 80, 128), 'corner not clipped'
        assert after.getpixel((f['x'] + 160, f['y'] + 120)) == (255, 255, 255), 'body changed'
        configure('auto')
        assert pixels().getpixel(outside) == before.getpixel(outside), 'automatic shadow policy changed'
        print('PASS: forced corners clear client shadows without replacement; auto preserves them')
    finally:
        process.terminate()
        process.wait(timeout=5)
