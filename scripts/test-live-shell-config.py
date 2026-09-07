#!/usr/bin/env python3
"""Check live configuration inside the isolated run-gnome-shell.sh session."""
import json
import os
from pathlib import Path
import subprocess
import time

root = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin'
root.mkdir(parents=True, exist_ok=True)
config = root / 'gnoblin.conf'
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
            meta_window: {is_monitor_sized: () => false},
            set_scale() {}, show() {},
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
        } else if (!transition || 'x' in transition || 'scale_x' in transition ||
            transition.duration !== expected['minimize-duration']) {
            throw new Error(`${action}: fade must stay in place and use configured duration`);
        }
    }
}
'''.replace('EXPECTED', json.dumps(expected)))
    call('ReloadScripts')


check({'window-switcher': False, 'minimize-animation': 'fade', 'minimize-duration': 200})
config.write_text('[shell]\nwindow-switcher = true\nminimize-animation = none\n')
time.sleep(0.5)
check({'window-switcher': True, 'minimize-animation': 'none', 'minimize-duration': 200})
temporary = config.with_suffix('.tmp')
temporary.write_text('[shell]\nminimize-duration = 80\n')
temporary.replace(config)
time.sleep(0.5)
check({'window-switcher': False, 'minimize-animation': 'fade', 'minimize-duration': 80})
config.write_text('[shell]\nminimize-duration = invalid\n')
call('ReloadConfig', success=False)
check({'window-switcher': False, 'minimize-animation': 'fade', 'minimize-duration': 80})
config.write_text('[shell]\nminimize-duration = 50\n')
call('ReloadConfig')
check({'window-switcher': False, 'minimize-animation': 'fade', 'minimize-duration': 50})
config.unlink()
time.sleep(0.5)
check({'window-switcher': False, 'minimize-animation': 'fade', 'minimize-duration': 200})
print('PASS: live shell watcher, manual reload, invalid-file recovery, minimise/restore, switcher gate')
