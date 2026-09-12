#!/usr/bin/env python3
"""Check real blur-region transport, pixels, caching and damage expansion."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from PIL import Image, ImageChops, ImageStat

repo = Path(__file__).resolve().parents[1]
config = Path(os.environ['XDG_CONFIG_HOME'])
if not str(config).startswith('/tmp/gnoblin-gs.'):
    raise SystemExit('Run as GNOBLIN_TEST_DBUS_CLIENT in run-gnome-shell.sh')
root = config / 'gnoblin'
scripts = root / 'scripts'
scripts.mkdir(parents=True, exist_ok=True)
shutil.copy2(repo / 'src/scripts/compositor-bridge.js', scripts)
shutil.copytree(repo / 'src/scripts/lib', scripts / 'lib', dirs_exist_ok=True)
(root / 'init.lua').write_text('''return {
    shell = {['layer-animation'] = 'none'},
    ['window-rules'] = {{match = {layer = '^blur-target$'}, blur = 24}},
}
''')
fixture = config / 'blur-region-fixture'
fixture.mkdir()
source = Path(os.environ.get('BINGUX_SOURCE', repo.parent / 'bingux')) / 'shell/bingux'
for name in ['BlurRegion.qml','BlurRegions.qml','ShortcutSession.qml','CompositorEnvironment.qml']:
    shutil.copy2(source / name, fixture / name)
(fixture / 'qmldir').write_text('BlurRegion 1.0 BlurRegion.qml\nShortcutSession 1.0 ShortcutSession.qml\nsingleton BlurRegions 1.0 BlurRegions.qml\nsingleton CompositorEnvironment 1.0 CompositorEnvironment.qml\n')
(fixture / 'shell.qml').write_text('''import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
ShellRoot {
 id: root
 property bool cropped: false
 property color patchColor: "#eeeeee"
 property real boxX: 800
 IpcHandler {
  target: "test"
  function crop(value: bool): void { root.cropped = value; }
  function patch(value: string): void { root.patchColor = value; }
  function move(value: real): void { root.boxX = value; }
 }
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  color: "black"
  Grid { columns: 160; Repeater { model: 160 * 100
   Rectangle { required property int index; width: 8; height: 8; color: (index % 160 + Math.floor(index / 160)) % 2 ? "#eeeeee" : "#222222" }
  } }
 }
 PanelWindow {
  anchors { top: true; left: true } implicitWidth: 300; implicitHeight: 150
  WlrLayershell.layer: WlrLayer.Bottom
  color: "#123456"
  Rectangle { width: 30; height: 150; color: "white"
   SequentialAnimation on x { loops: Animation.Infinite
    NumberAnimation { from: 0; to: 270; duration: 1000 }
    NumberAnimation { from: 270; to: 0; duration: 1000 }
   }
  }
 }
 PanelWindow {
  anchors { top: true; left: true } margins.left: 900; margins.top: 300
  implicitWidth: 40; implicitHeight: 40
  WlrLayershell.layer: WlrLayer.Bottom
  color: root.patchColor
 }
 PanelWindow {
  id: panel
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "blur-target"
  color: "transparent"
  Rectangle { x: root.boxX; y: 200; width: 320; height: 320; radius: 24; color: "#80303030" }
  BlurRegion { window: panel; surfaceNamespace: "blur-target"
   region: root.cropped ? Qt.rect(root.boxX, 200, 320, 320) : Qt.rect(0, 0, panel.width, panel.height)
  }
 }
}
''')
# Sample counters from the actual native effect without scheduling paints.
(scripts / 'blur-region-observer.js').write_text('''import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
export default function(api) {
 const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
  const actor = global.get_window_actors().find(a => Meta.gnoblin_layer_namespace(a.meta_window) === 'blur-target');
  const effect = actor?.get_effect('gnoblin-window-blur');
  if (effect) GLib.file_set_contents(GLib.build_filenamev([GLib.get_user_config_dir(), 'blur-observer.json']), JSON.stringify({region:actor._gnoblinBlurRegion, builds:effect.get_cache_build_count(), repaints:effect.get_repaint_count(), maskPaints:effect.get_mask_paint_count(), fullRedraw:!!(Clutter.get_debug_flags()[1] & Clutter.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS)}));
  return GLib.SOURCE_CONTINUE;
 });
 api._disposers.push(() => GLib.source_remove(timer));
}
''')
subprocess.run([str(repo / 'src/tools/gnoblinctl'), 'script', 'reload'], check=True)
log = (fixture / 'runtime.log').open('w')
qs = os.environ.get('GNOBLIN_QS', 'qs')
proc = subprocess.Popen([qs, '-p', str(fixture)], stdout=log, stderr=log)
def state():
    return json.loads((config / 'blur-observer.json').read_text())
def wait_for(predicate):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            result = state()
            if predicate(result): return result
        except (FileNotFoundError, json.JSONDecodeError): pass
        time.sleep(.1)
    raise AssertionError('Timed out: ' + repr(state()))
def ipc(method, *args):
    subprocess.run([qs, 'ipc', '-p', str(fixture), 'call', 'test', method, *map(str,args)], check=True)
    time.sleep(.4)
def capture(name):
    path = fixture / (name+'.png')
    subprocess.run(['grim', str(path)], check=True)
    return Image.open(path).convert('RGB')
try:
    initial = wait_for(lambda s: s.get('region') == [0,0,1280,800])
    assert not initial['fullRedraw'], initial
    time.sleep(.5)
    full = capture('full')
    ipc('crop', 'true')
    bounded = wait_for(lambda s: s.get('region') == [800,200,320,320] and s['maskPaints'] > initial['maskPaints'])
    crop = capture('cropped')
    delta = ImageStat.Stat(ImageChops.difference(full.crop((800,200,1120,520)), crop.crop((800,200,1120,520))))
    assert max(delta.mean) < 1.5, delta.mean
    time.sleep(.5)  # Let screencopy repair damage settle before measuring.
    before = state()
    time.sleep(1.5)
    after = state()
    assert after['repaints'] <= before['repaints'] + 1, (before,after)
    assert after['builds'] == before['builds'], (before,after)
    ipc('patch', '#ff0000')
    changed = wait_for(lambda s: s['repaints'] > after['repaints'])
    red = capture('red')
    patch_delta = ImageStat.Stat(ImageChops.difference(crop.crop((900,300,940,340)), red.crop((900,300,940,340))))
    assert max(patch_delta.mean) > 10, patch_delta.mean
    # A forced full redraw is the reference for the partially damaged result.
    ipc('crop', 'false')
    reference = capture('reference')
    delta = ImageStat.Stat(ImageChops.difference(red.crop((800,200,1120,520)), reference.crop((800,200,1120,520))))
    assert max(delta.mean) < 1.5, delta.mean
    before_move = state()
    ipc('move', 700)
    ipc('crop', 'true')
    wait_for(lambda s: s.get('region') == [700,200,320,320] and s['maskPaints'] > before_move['maskPaints'])
    moved = capture('moved')
    ipc('crop', 'false')
    moved_reference = capture('moved-reference')
    delta = ImageStat.Stat(ImageChops.difference(moved, moved_reference).crop((650,180,1150,550)))
    assert max(delta.mean) < 1.5, delta.mean
    print('PASS: cropped pixels, damage expansion, cache reuse outside damage, native region transport and movement', initial,bounded,after, flush=True)
finally:
    proc.terminate(); proc.wait(timeout=5); log.close()
    for image in fixture.glob('*.png'):
        shutil.copy2(image, Path('/tmp') / ('gnoblin-blur-' + image.name))
    print((fixture / 'runtime.log').read_text()[-2500:])
