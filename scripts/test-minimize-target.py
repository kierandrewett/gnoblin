#!/usr/bin/env python3
"""Run under run-gnome-shell.sh with Quickshell installed; use only its private session."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

if sys.argv[1:] == ['--app']:
    import gi
    gi.require_version('Gtk', '4.0')
    from gi.repository import Gtk, GLib
    Gtk.init()
    window = Gtk.Window(title='Gnoblin target test', default_width=400, default_height=300)
    window.set_child(Gtk.Label(label='This application must survive its dock'))
    window.present()
    GLib.MainLoop().run()
    sys.exit(0)

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
(root / 'scripts').mkdir(parents=True, exist_ok=True)
report = root / 'target.json'
easing = root / 'easing.json'
probe = root / 'scripts/target-probe.js'
probe.write_text('''
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
export default function () {
    const actor = global.get_window_actors().find(a => a.meta_window.get_title() === 'Gnoblin target test');
    if (!actor)
        return;
    const [set, rect] = actor.meta_window.get_icon_geometry();
    GLib.file_set_contents(REPORT, JSON.stringify({set, x: rect.x, y: rect.y,
        width: rect.width, height: rect.height, minimized: actor.meta_window.minimized,
        visible: actor.visible, mapping: Main.wm._mapping.has(actor)}));
    if (!actor._targetProbeInstalled) {
        actor._targetProbeInstalled = true;
        const original = actor.ease;
        actor.ease = function (params) {
            GLib.file_set_contents(EASING, JSON.stringify({x: params.x, y: params.y,
                scale_x: params.scale_x, scale_y: params.scale_y}));
            return original.call(this, params);
        };
    }
}
'''.replace('REPORT', json.dumps(str(report))).replace('EASING', json.dumps(str(easing))))


def wait_for(operation, description):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = operation()
        if result:
            return result
        time.sleep(0.1)
    raise RuntimeError(f'timeout: {description}')


def snapshot():
    subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.gnoblin.Shell',
                    '--object-path', '/org/gnoblin/Shell', '--method',
                    'org.gnoblin.Shell.ReloadScripts'], check=True, capture_output=True)
    return json.loads(report.read_text()) if report.exists() else None


app = subprocess.Popen([sys.executable, __file__, '--app'])
dock = None
try:
    wait_for(snapshot, 'application is mapped')
    fixture = Path(__file__).resolve().parent.parent / 'tests/minimize-dock/shell.qml'
    log = (root / 'dock.log').open('w')
    dock = subprocess.Popen(['qs', '-p', str(fixture)], stdout=log, stderr=subprocess.STDOUT)

    def ipc(method, *args):
        result = subprocess.run(['qs', 'ipc', '--pid', str(dock.pid), 'call', 'dock', method,
                                 *map(str, args)], text=True, capture_output=True)
        return result.stdout.strip() if result.returncode == 0 else ''

    wait_for(lambda: ipc('setTarget', 10, 20) == 'ready', 'dock sets a target')
    first = wait_for(lambda: (s if (s := snapshot())['set'] else None), 'native icon geometry')
    assert first['width'] == 32 and first['height'] == 40, first
    ipc('setTarget', 70, 30)
    second = wait_for(lambda: (s if (s := snapshot())['x'] == first['x'] + 60 else None), 'changed icon')
    assert second['y'] == first['y'] + 10, second
    ipc('moveDock')
    moved = wait_for(lambda: (s if (s := snapshot())['y'] == second['y'] - 50 else None), 'dock moves')
    ipc('restore')
    wait_for(lambda: (s := snapshot())['visible'] and not s['mapping'], 'application map completes')
    easing.unlink(missing_ok=True)
    ipc('minimize')
    wait_for(lambda: snapshot()['minimized'], 'application minimizes')
    wait_for(easing.exists, 'minimise animation starts')
    transition = json.loads(easing.read_text())
    assert transition['x'] == moved['x'] and transition['y'] == moved['y'], transition
    ipc('restore')
    wait_for(lambda: not snapshot()['minimized'], 'application restores')
    ipc('clearTarget')
    wait_for(lambda: not snapshot()['set'], 'dock clears target')
    assert ipc('setTarget', 10, 20) == 'ready'
    wait_for(lambda: snapshot()['set'], 'dock restores target')
    dock.kill()
    dock.wait(timeout=5)
    wait_for(lambda: not snapshot()['set'], 'dock crash clears target')
    assert app.poll() is None
    print('PASS: QML dock target, changed icon, moved dock, real minimise endpoint, restore, clear and crash cleanup')
finally:
    for child in [dock, app]:
        if child and child.poll() is None:
            child.terminate()
            child.wait(timeout=5)
