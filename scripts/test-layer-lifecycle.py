#!/usr/bin/env python3
"""Inspect compositor frames from a client with no animation code."""
import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-'), 'Use the private test session'
root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
(root / 'scripts').mkdir(parents=True, exist_ok=True)
report = root / 'layer-report.json'
motion = root / 'motion.txt'
motion.write_text('on')
config = root / 'gnoblin.toml'
(root / 'scripts/layer-probe.js').write_text('''
import Meta from 'gi://Meta';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
export default function enable(api) {
    const entries = [];
    const actors = new Map();
    const settings = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
    let animations = true;
    settings.set_boolean('enable-animations', animations);
    const flush = () => GLib.file_set_contents(REPORT, JSON.stringify(entries));
    const sample = actor => [actor.translation_x, actor.translation_y, actor.opacity];
    const record = (actor, phase) => {
        const entry = {phase, namespace: Meta.gnoblin_layer_namespace(actor.meta_window),
            frames: [sample(actor)]};
        entries.push(entry);
        actors.set(actor, entry);
        flush();
    };
    const map = global.window_manager.connect_after('map', (_wm, actor) => {
        if (!Meta.gnoblin_layer_namespace(actor.meta_window)?.startsWith('animation-test-')) return;
        record(actor, 'map');
        actor.connect('destroy', () => {
            if (actors.has(actor)) actors.get(actor).destroyed = true;
            actors.delete(actor);
            flush();
        });
    });
    const destroy = global.window_manager.connect_after('destroy', (_wm, actor) => {
        if (actors.has(actor)) record(actor, 'destroy');
    });
    const tick = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
        const enabled = new TextDecoder().decode(GLib.file_get_contents(MOTION)[1]) === 'on';
        if (enabled !== animations) {
            animations = enabled;
            settings.set_boolean('enable-animations', enabled);
        }
        for (const [actor, entry] of actors)
            if (entry.frames.length < 90) entry.frames.push(sample(actor));
        flush();
        return GLib.SOURCE_CONTINUE;
    });
    api._disposers.push(() => {
        GLib.source_remove(tick);
        global.window_manager.disconnect(map);
        global.window_manager.disconnect(destroy);
    });
}
'''.replace('REPORT', json.dumps(str(report))).replace('MOTION', json.dumps(str(motion))))
subprocess.run(['gnoblinctl', 'reload-scripts'], check=True)

cases = [(13, 'slide', 240), (14, 'slide', 180), (7, 'slide', 240),
         (11, 'slide', 240), (5, 'slide', 240), (15, 'slide', 240),
         (13, 'fade', 240), (13, 'none', 240), (13, 'slide', 0),
         (13, 'slide', 240), (13, 'slide', 240)]
for index, (anchor, animation, duration) in enumerate(cases):
    reduced_motion = index == 9
    interrupted = index == 10
    motion.write_text('off' if reduced_motion else 'on')
    config.write_text(f'[shell]\nlayer-animation="{animation}"\nlayer-duration={duration}\nlayer-easing="ease-out-cubic"\n')
    time.sleep(.35)
    name = f'animation-test-{index}'
    qml = root / f'layer-{index}.qml'
    qml.write_text('''import QtQuick
import Quickshell
import Quickshell.Wayland
PanelWindow {
 id: panel
 implicitWidth: 200; implicitHeight: 80; color: "#333333"
 ANCHORS
 WlrLayershell.layer: WlrLayer.Top
 WlrLayershell.namespace: "NAMESPACE"
 property int step: 0
 Timer { interval: INTERVAL; running: true; repeat: true
  onTriggered: { panel.step++; if (panel.step === 3) Qt.quit(); else panel.visible = panel.step === 2; }
 }
}
'''.replace('INTERVAL', '80' if interrupted else '600').replace('NAMESPACE', name).replace('ANCHORS', '\n'.join(
        f'anchors.{edge}: true' for bit, edge in [(1, 'top'), (2, 'bottom'), (4, 'left'), (8, 'right')] if anchor & bit)))
    result = subprocess.run(['qs', '-p', str(qml)], capture_output=True, text=True, timeout=6)
    assert result.returncode == 0, result.stderr
    time.sleep(.4)
    entries = [entry for entry in json.loads(report.read_text()) if entry['namespace'] == name]
    mapped = [entry for entry in entries if entry['phase'] == 'map']
    assert len(mapped) == 2, (name, entries)
    enabled = animation != 'none' and duration > 0 and not reduced_motion
    for entry in mapped:
        frames = entry['frames']
        first, last = frames[0], frames[-1]
        if not interrupted:
            assert last == [0, 0, 255], (name, 'not settled', last)
        if not enabled:
            assert all(frame == [0, 0, 255] for frame in frames), (name, frames)
        elif animation == 'fade' or anchor == 15:
            assert first == [0, 0, 0] and any(0 < frame[2] < 255 for frame in frames), (name, frames)
        else:
            x, y, opacity = first
            assert opacity == 255, 'A slide must remain opaque like a notification'
            assert (y < 0 if anchor in (13, 5) else y > 0 if anchor == 14 else y == 0), (name, first)
            assert (x < 0 if anchor in (7, 5) else x > 0 if anchor == 11 else x == 0), (name, first)
            assert any(0 < abs(frame[0]) + abs(frame[1]) < abs(x) + abs(y) for frame in frames), (name, 'no intermediate frames')
    if enabled:
        exits = [entry for entry in entries if entry['phase'] == 'destroy']
        assert exits and all(entry.get('destroyed') for entry in exits), (name, 'exit not completed')
        assert any(len(entry['frames']) > 2 for entry in exits), (name, 'no exit frames')
        if interrupted:
            assert all(entry['frames'][0][1] < 0 for entry in exits), (
                'Interrupted entrance jumped to its final position', [entry['frames'][0] for entry in exits])
    print(f'PASS: anchor={anchor} {animation} duration={duration} reduced={reduced_motion} interrupted={interrupted}: frames and cleanup')
