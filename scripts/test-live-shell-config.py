#!/usr/bin/env python3
"""Check live configuration inside the isolated run-gnome-shell.sh session."""
import json
import os
from pathlib import Path
import subprocess
import time

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
root.mkdir(parents=True, exist_ok=True)
config = root / 'gnoblin.toml'
scripts = root / 'scripts'
scripts.mkdir(exist_ok=True)
probe = scripts / 'config-test.js'


def call(method, success=True):
    result = subprocess.run([
        'gdbus', 'call', '--session', '--dest', 'org.gnoblin.Shell',
        '--object-path', '/org/gnoblin/Shell', '--method',
        f'org.gnoblin.Shell.{method}',
    ], capture_output=True, text=True)
    if (result.returncode == 0) != success:
        raise RuntimeError(result.stdout + result.stderr)


def check(expected):
    probe.write_text('''
import * as Config from 'resource:///org/gnome/shell/ui/components/gnoblinConfig.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
export default function () {
    const expected = EXPECTED;
    for (const [key, value] of Object.entries(expected)) {
        if (Config.settings[key] !== value)
            throw new Error(`${key}: ${Config.settings[key]} != ${value}`);
    }
    // A disabled switcher must return before accessing the popup arguments.
    if (!expected['window-switcher'])
        Main.wm._startSwitcher(null, null, null, {get_name: () => 'switch-applications'});
    for (const action of ['minimize', 'unminimize']) {
        let transition;
        let completed = false;
        const actor = {
            meta_window: {
                is_monitor_sized: () => false,
                get_monitor: () => 0,
                get_icon_geometry: () => [true, {x: 400, y: 700, width: 40, height: 40}],
                get_buffer_rect: () => ({x: 100, y: 100}),
            },
            width: 800, height: 600,
            set_scale() {}, set_position() {}, show() {},
            ease(params) { transition = params; },
        };
        const wm = {
            _shouldAnimateActor: () => true,
            _minimizing: new Set(), _unminimizing: new Set(),
        };
        Main.wm[`_${action}Window`].call(wm, {
            [`completed_${action}`]() { completed = true; },
        }, actor);
        if (expected['minimize-animation'] === 'none') {
            if (!completed || transition)
                throw new Error(`${action}: none must complete without animation`);
        } else {
            if (!transition || transition.duration !== expected['minimize-duration'])
                throw new Error(`${action}: configured duration`);
            if (action === 'minimize' && (transition.x !== 400 || transition.y !== 700))
                throw new Error('zoom must target the icon rectangle');
        }
    }
}
'''.replace('EXPECTED', json.dumps(expected)))
    call('ReloadScripts')


check({'window-switcher': False, 'minimize-animation': 'zoom', 'minimize-duration': 200})
config.write_text('[shell]\nwindow-switcher = true\nminimize-animation = "none"\n')
time.sleep(0.5)
check({'window-switcher': True, 'minimize-animation': 'none', 'minimize-duration': 200})
temporary = config.with_suffix('.tmp')
temporary.write_text('[shell]\nminimize-duration = 80\n')
temporary.replace(config)
time.sleep(0.5)
check({'window-switcher': False, 'minimize-animation': 'zoom', 'minimize-duration': 80})
config.write_text('[shell]\nminimize-duration = invalid\n')
call('ReloadConfig', success=False)
check({'window-switcher': False, 'minimize-animation': 'zoom', 'minimize-duration': 80})
config.write_text('[shell]\nminimize-duration = 50\n')
call('ReloadConfig')
check({'window-switcher': False, 'minimize-animation': 'zoom', 'minimize-duration': 50})
config.unlink()
time.sleep(0.5)
check({'window-switcher': False, 'minimize-animation': 'zoom', 'minimize-duration': 200})
config.write_text('[shell]\nosd = false\nscreenshot = false\n')
call('ReloadConfig')
for feature in ['osd', 'screenshot']:
    result = subprocess.check_output(['gdbus', 'call', '--session', '--dest', 'org.gnoblin.Shell', '--object-path', '/org/gnoblin/Shell', '--method', 'org.gnoblin.Shell.GetFeature', feature], text=True)
    assert result.strip() == '(false,)', result
config.write_text('[shell]\nosd = true\nscreenshot = true\n')
time.sleep(0.5)
for feature in ['osd', 'screenshot']:
    result = subprocess.check_output(['gdbus', 'call', '--session', '--dest', 'org.gnoblin.Shell', '--object-path', '/org/gnoblin/Shell', '--method', 'org.gnoblin.Shell.GetFeature', feature], text=True)
    assert result.strip() == '(true,)', result
# Named commands run once, including after they exit and the file reloads.
started = root / 'autostart-count'
command = ['sh', '-c', 'echo started >> "$1"', 'autostart-test', str(started)]
config.write_text('[[autostart]]\nname = "probe"\ncommand = ' + json.dumps(command) + '\n')
call('ReloadConfig')
time.sleep(0.3)
call('ReloadConfig')
time.sleep(0.3)
assert started.read_text().splitlines() == ['started']
config.write_text(config.read_text() + '\n[[autostart]]\nname = "second"\ncommand = ' + json.dumps(command) + '\n')
time.sleep(0.5)
assert started.read_text().splitlines() == ['started', 'started']
print('PASS: autostart runs once per name and accepts newly added entries')
print('PASS: live feature configuration')
print('PASS: live shell watcher, manual reload, invalid-file recovery, minimise/restore, switcher gate')
