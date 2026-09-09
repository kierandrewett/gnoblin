#!/usr/bin/env python3
"""Native pixel checks. Run as GNOBLIN_TEST_DBUS_CLIENT in an isolated shell."""
import json
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')
root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
root.mkdir(parents=True, exist_ok=True)
config = root / 'corner-test.json'
def configure(value):
    temporary = config.with_suffix('.tmp')
    temporary.write_text(json.dumps(value))
    temporary.replace(config)
    time.sleep(.4)
configure({'radius': 0})
(root / 'scripts').mkdir(exist_ok=True)
(root / 'scripts/corner-test.js').write_text((Path(__file__).resolve().parents[1] / 'tests/window-corners-native.js').read_text())
subprocess.run(['gdbus','call','--session','--dest','org.gnoblin.Shell','--object-path','/org/gnoblin/Shell','--method','org.gnoblin.Shell.ReloadScripts'],check=True)
time.sleep(.3)

def capture():
    path = root / 'corner-screen.png'
    subprocess.run(['grim', str(path)],check=True)
    return Image.open(path).convert('RGB')

def scene(radius, opacity):
    qml = root / 'corners.qml'
    qml.write_text('''import QtQuick
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
'''.replace('RADIUS',str(radius)).replace('OPACITY',str(opacity)))
    proc = subprocess.Popen(['qs','-p',str(qml)],stdout=subprocess.DEVNULL,stderr=(root/'client.log').open('w'),env={**os.environ,'QT_WAYLAND_DISABLE_WINDOWDECORATION':'1'})
    time.sleep(.8)
    frames=json.loads((root/'corner-frames.json').read_text())
    assert len(frames)==1, frames
    frame=frames[0]['frame']
    return proc, (frame['x'],frame['y'],frame['x']+frame['width'],frame['y']+frame['height'])

for shape, opacity in ((0, 1), (40, 1), (120, 1), (0, .5)):
    configure({'radius':0})
    proc, box = scene(shape, opacity)
    try:
        baseline = capture()
        plain = baseline.crop(box)
        configure({'radius':24,'mode':'auto'})
        rounded = capture().crop(box)
        if shape == 0:
            assert max(plain.getpixel((2,2)))>180
            assert sum(rounded.getpixel((2,2))) < sum(plain.getpixel((2,2)))-80, 'square corner not clipped'
            assert rounded.getpixel((160,120))==plain.getpixel((160,120)), 'interior changed'
            configure({'radius':24,'smoothing':.6,'mode':'auto'})
            smooth=capture().crop(box)
            assert sum(ImageStat.Stat(ImageChops.difference(rounded,smooth)).sum)>100, 'smoothing had no effect'
            configure({'radius':24,'mode':'force','border-width':3,'border-color':'#ff0000ff'})
            border=capture().crop(box)
            assert border.getpixel((160,1))[0]>200 and border.getpixel((160,1))[1]<60, 'border not rendered'
            configure({'radius':24,'mode':'auto','shadow':{'blur':16,'opacity':.6}})
            shadow=capture()
            shadow.save('/tmp/gnoblin-corner-shadow.png')
            edge=(box[2]+4, box[1]+120)
            assert sum(shadow.getpixel(edge)) < sum(baseline.getpixel(edge))-10, ('custom shadow not visible',shadow.getpixel(edge),baseline.getpixel(edge))
            assert shadow.getpixel((box[0]+160,box[1]+120)) == baseline.getpixel((box[0]+160,box[1]+120)), 'shadow painted over contents'
            for action, keep in [('maximize','keep-maximized'), ('fullscreen','keep-fullscreen')]:
                configure({'radius':24,'_action':action})
                assert not json.loads((root/'corner-frames.json').read_text())[0]['effect'], action+' did not suspend rounding'
                configure({'radius':24,keep:True})
                assert json.loads((root/'corner-frames.json').read_text())[0]['effect'], keep+' did not restore rounding'
                configure({'radius':24,'_action':'un'+action})
        else:
            assert max(ImageStat.Stat(ImageChops.difference(plain,rounded)).mean)<.2, 'automatic mode modified an existing rounded/shaped corner'
        configure({'radius':0})
        restored=capture().crop(box)
        assert max(ImageStat.Stat(ImageChops.difference(plain,restored)).mean)<.2, 'removing rule changed window pixels'
    finally:
        proc.terminate();proc.wait(timeout=5)
        time.sleep(.2)
print('PASS: native circular/smoothed corners, border, shadows, translucent contents, maximised/fullscreen policy, shaped/already-rounded bypass and removal')
