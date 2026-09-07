#!/usr/bin/env python3
"""Exercise the switcher with virtual keyboard events in a private session."""
import ast
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')
repo = Path(__file__).resolve().parent.parent
scripts = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin/scripts'
scripts.mkdir(parents=True, exist_ok=True)
shutil.copy2(repo / 'src/scripts/window-switcher.js', scripts)
(scripts / 'switcher-test.js').write_text('''
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
export default function enable(api) {
 // A physical keyboard survives shell script reloads. Keep this private test
 // device for the session too, so reload exercises the switcher's own cleanup.
 global.__switcherTestKeyboard ??= global.stage.context.get_backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
 const device = global.__switcherTestKeyboard;
 let released = 0, latency = 0;
 const focus = global.display.connect('notify::focus-window', () => {
  if (released && global.display.focus_window) latency = GLib.get_monotonic_time() - released;
 });
 const impl = Gio.DBusExportedObject.wrapJSObject(`<node><interface name="org.gnoblin.SwitcherTest">
 <method name="Key"><arg type="u" direction="in"/><arg type="b" direction="in"/></method>
 <method name="State"><arg type="s" direction="out"/></method>
 <method name="Focus"><arg type="s" direction="in"/></method>
 </interface></node>`, {
  Key(code, down) {
   if (code === 56 && !down) { released = GLib.get_monotonic_time(); latency = 0; }
   device.notify_key(GLib.get_monotonic_time(), code, down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
  },
  Focus(title) {
   const window = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null).find(w => w.title === title);
   Main.activateWindow(window, global.get_current_time());
  },
  State() {
   const actor = Main.uiGroup.get_children().find(a => a.name === 'gnoblin-window-switcher');
   const label = actor?.get_first_child()?.get_last_child();
   return JSON.stringify({focus: global.display.focus_window?.title ?? null, shown: !!actor?.visible && actor.opacity > 0,
    visible: !!actor?.visible, label: label?.text, latency,
    windows: global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null).map(w => w.title)});
  },
 });
 impl.export(Gio.DBus.session, '/org/gnoblin/SwitcherTest');
 const name = Gio.bus_own_name(Gio.BusType.SESSION, 'org.gnoblin.SwitcherTest', Gio.BusNameOwnerFlags.NONE, null, null, null);
 api._disposers.push(() => {
  global.display.disconnect(focus); impl.unexport(); Gio.bus_unown_name(name);
 });
}
''')


def reload():
    subprocess.run([str(repo / 'src/tools/gnoblinctl'), 'reload-scripts'], check=True)


def call(method, *args):
    result = subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.gnoblin.SwitcherTest',
        '--object-path', '/org/gnoblin/SwitcherTest', '--method', 'org.gnoblin.SwitcherTest.' + method,
        *map(str, args)], check=True, text=True, capture_output=True)
    return ast.literal_eval(result.stdout)


def state():
    return json.loads(call('State')[0])


def wait_for(predicate, timeout=3):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = state()
        if predicate(value):
            return value
        time.sleep(.01)
    raise AssertionError(value)


def key(code, down):
    call('Key', code, 'true' if down else 'false')


def tap(code):
    key(code, True)
    key(code, False)


reload()
apps = []
try:
    for title in ['Switcher One', 'Switcher Two', 'Switcher Three']:
        apps.append(subprocess.Popen(['foot', '--app-id=gnoblin-switcher-test', '--title=' + title, 'sleep', '90'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        wait_for(lambda s: s['focus'] == title)
    assert state()['windows'][:3] == ['Switcher Three', 'Switcher Two', 'Switcher One']
    key(56, True)
    tap(15)
    shown = wait_for(lambda s: s['shown'])
    time.sleep(.1)
    subprocess.run(['grim', '/tmp/gnoblin-switcher-preview.png'], check=True)
    assert shown['focus'] == 'Switcher Three', shown
    assert shown['label'].startswith('Switcher Two'), shown
    tap(15)
    wait_for(lambda s: s['label'].startswith('Switcher One'))
    key(56, False)
    wait_for(lambda s: s['focus'] == 'Switcher One' and not s['visible'])
    key(56, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    tap(1)
    key(56, False)
    assert state()['focus'] == 'Switcher One' and not state()['visible'], 'Escape must not change focus'
    key(56, True)
    key(42, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    assert state()['label'].startswith('Switcher Two'), state()
    key(42, False)
    key(56, False)
    wait_for(lambda s: s['focus'] == 'Switcher Two')
    timings = []
    for _ in range(20):
        previous = state()['focus']
        key(56, True)
        tap(15)
        key(56, False)
        result = wait_for(lambda s: s['focus'] != previous and not s['visible'])
        assert result['latency'] > 0, result
        timings.append(result['latency'] / 1000)
    print(f'KEY RELEASE TO FOCUS: median {statistics.median(timings):.2f} ms; max {max(timings):.2f} ms (20 switches)')
    key(56, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    apps[0].terminate()
    apps[0].wait(timeout=3)
    wait_for(lambda s: 'Switcher One' not in s['windows'])
    key(56, False)
    wait_for(lambda s: not s['visible'] and s['focus'] in ['Switcher Two', 'Switcher Three'])
    key(56, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    reload()
    key(56, False)
    assert not state()['visible'], 'reload must release the switcher grab'
    config = scripts.parent / 'gnoblin.toml'
    original = config.read_text() if config.exists() else ''
    config.write_text(original + '\n[switcher]\nenabled=false\n')
    time.sleep(.3)
    key(56, True)
    tap(15)
    time.sleep(.15)
    assert not state()['visible'], 'disabled switcher still took the binding'
    key(56, False)
    config.write_text(original + '\n[switcher]\nshow-delay=1\n')
    time.sleep(.3)
    key(56, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    tap(1)
    key(56, False)
    config.write_text(original + '\n[switcher]\nshow-delay=-1\n')
    time.sleep(.3)
    key(56, True)
    tap(15)
    wait_for(lambda s: s['shown'])
    tap(1)
    key(56, False)
    print('PASS: real Alt+Tab, stable MRU, reverse, Escape, rapid switching, closed windows, script/config reload and invalid config recovery')
finally:
    for app in apps:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=3)
