#!/usr/bin/env python3
"""Pixel regression for masked layer blur in an isolated Gnoblin session."""
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
root.mkdir(parents=True, exist_ok=True)
if not str(root).startswith('/tmp/gnoblin-gs.'):
    raise SystemExit('Run inside run-gnome-shell.sh')
scripts = root / 'scripts'
scripts.mkdir(exist_ok=True)
(scripts / 'surface-fade.js').write_text('''import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
export default function(api) {
 let previous = '';
 const path = GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin', 'fade']);
 const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 30, () => {
  try {
   const [, data] = GLib.file_get_contents(path);
   const value = new TextDecoder().decode(data);
   const actor = global.get_window_actors().find(a => Meta.gnoblin_layer_namespace(a.meta_window) === 'effect-mask');
   if (actor && value !== previous) { actor.opacity = Math.round(Number(value) * 255); previous = value; }
  } catch (_) {}
  return GLib.SOURCE_CONTINUE;
 });
 api._disposers.push(() => GLib.source_remove(timer));
}
''')
repo = Path(__file__).resolve().parents[1]
subprocess.run([str(repo / 'src/tools/gnoblinctl'), 'reload-scripts'], check=True)

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
  implicitWidth: 1280; implicitHeight: 800
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "effect-mask"
  color: "transparent"
  Rectangle { x: 64; y: 64; width: 192; height: 96; radius: 16; color: "#80303030" }
 }
}
''')
config = root / 'gnoblin.toml'
def configure(blur, opacity=1):
    config.write_text(f'[shell]\nlayer-animation="none"\n[[window-rules]]\nmatch.layer="^effect-mask$"\nblur={blur}\nopacity={opacity}\n')
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
    # Reference is the fully composed blurred panel, faded as a single image.
    (root / 'fade').write_text('0')
    time.sleep(.2)
    background = capture('background.png')
    for alpha in (1, .75, .5, .25, .1, 0, .5, 1):
        (root / 'fade').write_text(str(alpha))
        time.sleep(.2)
        actual = capture(f'fade-{alpha}.png')
        factor = round(alpha * 255) / 255
        expected = Image.blend(background, after, factor)
        # Include the antialiased silhouette and the surrounding wallpaper.
        delta = ImageStat.Stat(ImageChops.difference(actual, expected).crop((56,56,264,168)))
        assert max(delta.mean) < 2, (alpha, delta.mean)
        assert max(high for low, high in delta.extrema) <= 4, (alpha, delta.extrema)
    print('PASS: Gaussian background and client fade as one surface at 8 opacity samples, including reversal', flush=True)
finally:
    proc.terminate()
    proc.wait(timeout=5)
