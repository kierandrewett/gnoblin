#!/usr/bin/env python3
"""Measure a fixed animated backdrop in the private Gnoblin compositor."""
import json
import os
from pathlib import Path
import subprocess
import time

config = Path(os.environ['XDG_CONFIG_HOME'])
if not str(config).startswith('/tmp/gnoblin-gs.'):
    raise SystemExit('Run as GNOBLIN_TEST_DBUS_CLIENT in run-gnome-shell.sh')
root = config / 'gnoblin'
root.mkdir(exist_ok=True)
(root / 'gnoblin.toml').write_text('[shell]\nlayer-animation="none"\n[[window-rules]]\nmatch.layer="^blur-benchmark$"\nblur=24\n')
fixture = root / 'blur-benchmark.qml'
fixture.write_text('''import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  color: "#102030"
  Rectangle {
   width: 600; height: 800; color: "#eeeeee"
   SequentialAnimation on x {
    loops: Animation.Infinite
    NumberAnimation { from: 0; to: 680; duration: 2500 }
    NumberAnimation { from: 680; to: 0; duration: 2500 }
   }
  }
 }
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "blur-benchmark"
  color: "transparent"
  Rectangle { x: 800; y: 0; width: 480; height: 800; color: "#80303030" }
 }
}
''')
script_dir = root / 'scripts'
script_dir.mkdir(exist_ok=True)
(script_dir / 'blur-benchmark.js').write_text('''import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
export default function(api) {
 let frames = 0, paint = 0, start = 0;
 function cpu() { const [,bytes] = GLib.file_get_contents('/proc/self/stat'); const f = new TextDecoder().decode(bytes).split(') ')[1].split(' '); return Number(f[11]) + Number(f[12]); }
 const initial = cpu();
 const before = global.stage.connect('before-paint', () => { start = GLib.get_monotonic_time(); });
 const after = global.stage.connect('after-paint', () => {frames++; paint += GLib.get_monotonic_time() - start;});
 let timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 5000, () => {
  const actor = global.get_window_actors().find(a => Meta.gnoblin_layer_namespace(a.meta_window) === 'blur-benchmark');
  const blur = actor?.get_effect('gnoblin-window-blur');
  GLib.file_set_contents(GLib.build_filenamev([GLib.get_user_config_dir(), 'blur-result.json']), JSON.stringify({frames, paintMs:paint/1000, cpuTicks:cpu()-initial, cacheBuilds:blur?.get_cache_build_count?.() ?? null}));
  timer = 0; return GLib.SOURCE_REMOVE;
 });
 api._disposers.push(() => {if(timer) GLib.source_remove(timer); global.stage.disconnect(before); global.stage.disconnect(after);});
}
''')
proc = subprocess.Popen([os.environ.get('GNOBLIN_QS', 'qs'), '-p', str(fixture)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(2)
    subprocess.run([str(Path(__file__).resolve().parents[1] / 'src/tools/gnoblinctl'), 'reload-scripts'], check=True, stdout=subprocess.DEVNULL)
    report = config / 'blur-result.json'
    deadline = time.monotonic() + 10
    while not report.exists() and time.monotonic() < deadline:
        time.sleep(.1)
    result = json.loads(report.read_text())
    assert result['frames'] > 30, result
    if os.environ.get('GNOBLIN_BLUR_REQUIRE_CACHE') == '1':
        assert result['cacheBuilds'] == 1, result
    print(json.dumps(result), flush=True)
    output = os.environ.get('GNOBLIN_BLUR_REPORT')
    if output:
        Path(output).write_text(json.dumps(result) + '\n')
finally:
    proc.terminate()
    proc.wait(timeout=5)
