#!/usr/bin/env python3
"""Compare translucent details over a checkerboard with a flat-backdrop reference."""
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
if not str(root).startswith('/tmp/gnoblin-gs.'):
    raise SystemExit('Run inside run-gnome-shell.sh')
root.mkdir(parents=True, exist_ok=True)
(root / 'init.lua').write_text("""return {
 shell = {['layer-animation'] = 'none'},
 ['window-rules'] = {{match = {layer = '^blur-details$'}, blur = 24, ['blur-ignore-shadows'] = true}},
}""")
icons = {name: os.environ.get(name, '') for name in ('STEAM_ICON', 'LOCALSEND_ICON')}
for name, path in icons.items():
    if path and not Path(path).is_file():
        raise SystemExit(f'{name} does not exist: {path}')
qml = root / 'details.qml'
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Io
ShellRoot {
 id: root
 property bool flat: false
 IpcHandler { target: "test"; function reference(value: bool): void { root.flat = value; } }
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  color: "#888888"
  Grid { visible: !root.flat; columns: 320
   Repeater { model: 320 * 200
    Rectangle { required property int index; width: 4; height: 4
     color: (index % 320 + Math.floor(index / 320)) % 2 ? "#eeeeee" : "#222222"
    }
   }
  }
 }
 PanelWindow {
  anchors { top: true; left: true }
  implicitWidth: 640; implicitHeight: 400
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "blur-details"
  color: "transparent"
  Rectangle { x: 32; y: 32; width: 576; height: 336; radius: 20; color: "#80303030"
   Rectangle { x: 30; y: 40; width: 500; height: 1; color: "white" }
   Rectangle { x: 30; y: 80; width: 500; height: 1; color: "#60ffffff" }
   Rectangle { x: 30; y: 100; width: 500; height: 2; color: "#b0303030" }
   Rectangle { x: 30; y: 120; width: 96; height: 96; radius: 20
    color: "transparent"; border.width: 1; border.color: "white"
   }
   Rectangle { x: 160; y: 120; width: 96; height: 96; radius: 48
    color: "#604080c0"; border.width: 2; border.color: "#aaffffff"
   }
   Image { x: 300; y: 140; width: 48; height: 48; source: "STEAM_ICON" }
   Image { x: 400; y: 140; width: 48; height: 48; source: "LOCALSEND_ICON" }
  }
 }
}
""".replace('STEAM_ICON', icons['STEAM_ICON'])
     .replace('LOCALSEND_ICON', icons['LOCALSEND_ICON']))
qs = os.environ.get('QS_TEST_BIN', 'qs')
log = (root / 'details.log').open('w')
proc = subprocess.Popen([qs, '-p', str(qml)], stdout=log, stderr=log)
output = Path(os.environ.get('BLUR_DETAIL_OUTPUT', '/tmp/gnoblin-blur-details'))
output.mkdir(parents=True, exist_ok=True)
def capture(name):
    path = output / (name + '.png')
    subprocess.run(['grim', str(path)], check=True)
    return Image.open(path).convert('RGB')
try:
    time.sleep(2)
    assert proc.poll() is None, (root / 'details.log').read_text()
    actual = capture('checker')
    subprocess.run([qs, 'ipc', '-p', str(qml), 'call', 'test', 'reference', 'true'], check=True)
    # A successful IPC reply does not mean that the new frame is on screen.
    deadline = time.monotonic() + 5
    while True:
        time.sleep(.1)
        reference = capture('reference')
        background_delta = ImageChops.difference(actual, reference).crop((800, 100, 900, 200))
        if max(ImageStat.Stat(background_delta).mean) > 50:
            break
        assert time.monotonic() < deadline, 'Background did not change to the reference'
    delta = ImageChops.difference(actual, reference)
    delta.save(output / 'difference.png')
    failures = []
    for name, box in {
        'opaque separator': (60, 70, 565, 76),
        'translucent separator': (60, 110, 565, 116),
        'same-colour separator': (60, 130, 565, 136),
        'border': (58, 148, 164, 252),
        'translucent icon': (188, 148, 292, 252),
        'Steam': (328, 168, 384, 224),
        'LocalSend': (428, 168, 484, 224),
    }.items():
        icon_key = {'Steam': 'STEAM_ICON', 'LocalSend': 'LOCALSEND_ICON'}.get(name)
        if icon_key and not icons[icon_key]:
            print(f'SKIP: {name}; set {icon_key} to test its installed image', flush=True)
            continue
        stat = ImageStat.Stat(delta.crop(box))
        peak = max(high for low, high in stat.extrema)
        print(f'{name}: maximum backdrop leak={peak}, mean={max(stat.mean):.3f}', flush=True)
        if peak > 4: failures.append((name, peak))
    assert not failures, failures
    print('PASS: no sharp backdrop leaks beside separators, borders or icon details', flush=True)
finally:
    proc.terminate()
    proc.wait(timeout=5)
    log.close()
    runtime_log = (root / 'details.log').read_text()
    if proc.returncode not in (0, -15) or 'Error' in runtime_log:
        print(runtime_log)
