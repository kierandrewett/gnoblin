#!/usr/bin/env python3
"""Exercise the Gnoblin developer console in a private compositor session.

Run with GNOBLIN_TEST_DBUS_CLIENT from scripts/run-gnome-shell.sh. The probe
uses the normal user-script host to inspect the real Shell actor and drives the
same Run binding that users use, rather than exposing a second evaluator over
D-Bus.
"""
import ast
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin' / 'scripts'
SCRIPTS.mkdir(parents=True, exist_ok=True)
MARKER = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin' / 'developer-console-result.json'


def gdbus(service, path, interface, method, *arguments, ok=True, timeout=5):
    result = subprocess.run([
        'gdbus', 'call', '--session', '--dest', service, '--object-path', path,
        '--method', f'{interface}.{method}', *map(str, arguments),
    ], capture_output=True, text=True, timeout=timeout)
    assert (result.returncode == 0) == ok, result.stdout + result.stderr
    return result


(SCRIPTS / 'developer-console-test.js').write_text(r'''
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const marker = GLib.build_filenamev([
    GLib.get_user_config_dir(), 'gnoblin', 'developer-console-result.json',
]);
let keyboard = null;

function writeMarker(value) {
    try {
        Gio.File.new_for_path(marker).replace_contents(
            new TextEncoder().encode(JSON.stringify(value)), null, false,
            Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    } catch (error) {
        logError(error, 'developer-console-test: writing marker failed');
    }
}

function actorName(actor) {
    if (!actor)
        return null;
    return actor.get_name?.() ?? actor.name ?? null;
}

function consoleState() {
    const console = Main.devConsole;
    const monitor = Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
    const children = Main.uiGroup?.get_children() ?? [];
    const focus = global.stage.get_key_focus();
    return {
        input: console?._entry.get_text() ?? '',
        attributes: console?._entry.clutter_text.get_attributes()?.to_string() ?? '',
        exists: Boolean(console),
        open: Boolean(console?.isOpen),
        visible: Boolean(console?.visible),
        parent: actorName(console?.get_parent()),
        x: console?.x ?? null,
        y: console?.y ?? null,
        width: console?.width ?? null,
        height: console?.height ?? null,
        monitorY: monitor?.y ?? null,
        monitorWidth: monitor?.width ?? null,
        monitorHeight: monitor?.height ?? null,
        focusEntry: Boolean(console && (focus === console._entry || focus === console._entry.clutter_text)),
        modalCount: Main.modalCount,
        topIndex: console ? children.indexOf(console) : -1,
        childCount: children.length,
        topSibling: actorName(children.at(-1)),
        inspectorVisible: Boolean(console?._transcript?.get_last_child()?.get_first_child()?.get_n_children() > 1),
        inspectorRows: console?._transcript?.get_last_child()?.get_first_child()?.get_last_child()?.get_n_children() ?? 0,
        transcriptRows: console?._transcript?.get_n_children() ?? 0,
        runDialogAbsent: Main.runDialog === null,
        lookingGlassAbsent: Main.lookingGlass === null,
        lookingGlassAlias: Boolean(console && Main.createLookingGlass() === console),
        sessionMode: Main.sessionMode.currentMode,
        locked: Boolean(Main.sessionMode.isLocked),
    };
}

function serialiseValue(value) {
    if (value === null || ['string', 'number', 'boolean'].includes(typeof value))
        return value;
    if (value === undefined)
        return null;
    return String(value);
}

function serialiseResult(row) {
    return {
        id: row.id,
        source: row.source,
        value: row.error ? null : serialiseValue(row.value),
        error: row.error ? {
            name: String(row.error.name ?? 'Error'),
            message: String(row.error.message ?? row.error),
            stack: String(row.error.stack ?? ''),
        } : null,
        durationMs: row.durationMs,
    };
}

function keyboardDevice() {
    return global.stage.context.get_backend().get_default_seat().create_virtual_device(
        Clutter.InputDeviceType.KEYBOARD_DEVICE);
}

export default function (api) {
    const iface = `<node><interface name="org.gnoblin.DeveloperConsoleTest">
      <method name="State"><arg type="s" direction="out"/></method>
      <method name="Open"><arg type="s" direction="out"/></method>
      <method name="RunBinding"><arg type="s" direction="out"/></method>
      <method name="Evaluate"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
      <method name="ExpandLua"><arg type="s" direction="out"/></method>
      <method name="InspectRich"><arg type="s" direction="out"/></method>
      <method name="Inspect"><arg type="s" direction="out"/></method>
      <method name="Key"><arg type="u" direction="in"/></method>
      <method name="Language"><arg type="s" direction="in"/></method>
      <method name="Type"><arg type="s" direction="in"/></method>
      <method name="Complete"><arg type="s" direction="out"/></method>
      <method name="Escape"><arg type="s" direction="out"/></method>
      <method name="StartLock"><arg type="s" direction="out"/></method>
    </interface></node>`;
    const object = Gio.DBusExportedObject.wrapJSObject(iface, {
        State() {
            return JSON.stringify(consoleState());
        },
        Open() {
            const opened = Main.openDevConsole();
            return JSON.stringify({opened, ...consoleState()});
        },
        RunBinding() {
            Main.devConsole?.close(true);
            keyboard ??= keyboardDevice();
            const timestamp = GLib.get_monotonic_time();
            keyboard.notify_keyval(timestamp, Clutter.KEY_Alt_L, Clutter.KeyState.PRESSED);
            keyboard.notify_keyval(timestamp, Clutter.KEY_F2, Clutter.KeyState.PRESSED);
            keyboard.notify_keyval(timestamp, Clutter.KEY_F2, Clutter.KeyState.RELEASED);
            keyboard.notify_keyval(timestamp, Clutter.KEY_Alt_L, Clutter.KeyState.RELEASED);
            return JSON.stringify(consoleState());
        },
        Evaluate(source) {
            const console = Main.createDevConsole();
            console.evaluate(source).then(row => {
                writeMarker({phase: 'evaluated', result: serialiseResult(row), state: consoleState()});
            }).catch(error => {
                writeMarker({phase: 'failed', error: String(error), state: consoleState()});
            });
            return JSON.stringify({scheduled: true});
        },
        ExpandLua() {
            const console = Main.devConsole;
            const group = console._transcript.get_last_child().get_last_child().get_first_child();
            group.get_first_child().emit('clicked', 1);
            const names = group.get_last_child().get_children().map(row => row.get_first_child()?.text ?? '');
            return JSON.stringify({names});
        },
        InspectRich() {
            const console = Main.devConsole;
            console.clear();
            let calls = 0;
            const prototype = {inherited: 9};
            const object = Object.assign(Object.create(prototype), {
                title: 'Inspector', nested: {ready: true}, items: [1, 2, {name: 'third'}],
                collection: new Map([['key', {value: 42}]]),
            });
            object.self = object;
            object[Symbol('token')] = 'symbol value';
            Object.defineProperty(object, 'computed', {get() { calls++; return {answer: 42}; }});
            Object.defineProperty(object, 'broken', {get() { throw new Error('getter failure'); }});
            console.inspectObject(object);
            const group = console._transcript.get_last_child().get_first_child();
            const properties = group.get_last_child();
            const before = calls;
            const rows = properties.get_children();
            const computed = rows.find(row => row.get_first_child()?.text === 'computed: ');
            computed.get_last_child().emit('clicked', 1);
            const after = calls;
            const broken = rows.find(row => row.get_first_child()?.text === 'broken: ');
            broken.get_last_child().emit('clicked', 1);
            const failure = broken.get_last_child().text;
            const names = rows.map(row => row.get_first_child()?.text ?? '');
            return JSON.stringify({before, after, failure, names});
        },
        Inspect() {
            const console = Main.createDevConsole();
            console.inspectObject(global.stage);
            return JSON.stringify(consoleState());
        },
        Key(key) {
            keyboard ??= keyboardDevice();
            const timestamp = GLib.get_monotonic_time();
            keyboard.notify_keyval(timestamp, key, Clutter.KeyState.PRESSED);
            keyboard.notify_keyval(timestamp, key, Clutter.KeyState.RELEASED);
        },
        Language(language) {
            Main.devConsole._languageTabs.get(language).emit('clicked', 1);
        },
        Type(text) {
            Main.devConsole._entry.set_text(text);
            Main.devConsole._entry.clutter_text.set_cursor_position(-1);
        },
        Complete() {
            const console = Main.createDevConsole();
            const labels = console._completions.get_children().map(child =>
                String(child.label ?? child.get_child?.()?.text ?? ''));
            const result = {...consoleState(), labels, completionVisible: console._completions.visible};
            return JSON.stringify(result);
        },
        Escape() {
            keyboard ??= keyboardDevice();
            const timestamp = GLib.get_monotonic_time();
            keyboard.notify_keyval(timestamp, Clutter.KEY_Escape, Clutter.KeyState.PRESSED);
            keyboard.notify_keyval(timestamp, Clutter.KEY_Escape, Clutter.KeyState.RELEASED);
            return JSON.stringify(consoleState());
        },
        StartLock() {
            const originalMode = Main.sessionMode.currentMode;
            if (!Main.sessionMode.isLocked)
                Main.sessionMode.pushMode('unlock-dialog');
            writeMarker({phase: 'locked', originalMode, state: consoleState()});
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                if (Main.sessionMode.currentMode === 'unlock-dialog')
                    Main.sessionMode.popMode('unlock-dialog');
                writeMarker({phase: 'restored', state: consoleState()});
                return GLib.SOURCE_REMOVE;
            });
            return JSON.stringify(consoleState());
        },
    });
    object.export(Gio.DBus.session, '/org/gnoblin/DeveloperConsoleTest');
    const owner = Gio.bus_own_name(
        Gio.BusType.SESSION, 'org.gnoblin.DeveloperConsoleTest',
        Gio.BusNameOwnerFlags.NONE, null, null, null);
    api._disposers.push(() => {
        object.unexport();
        Gio.bus_unown_name(owner);
        keyboard?.run_dispose();
        keyboard = null;
    });
}
''')


INTERFACE = 'org.gnoblin.DeveloperConsoleTest'
SERVICE = INTERFACE
PATH = '/org/gnoblin/DeveloperConsoleTest'


def call(method, *arguments, ok=True, timeout=5):
    return gdbus(SERVICE, PATH, INTERFACE, method, *arguments, ok=ok, timeout=timeout)


def value(result):
    return ast.literal_eval(result.stdout)[0]


def state():
    return json.loads(value(call('State')))


def wait_for(predicate, timeout=5):
    deadline = time.monotonic() + timeout
    current = None
    while time.monotonic() < deadline:
        current = state()
        if predicate(current):
            return current
        time.sleep(.05)
    raise AssertionError(current)


def wait_marker(phase, timeout=5):
    deadline = time.monotonic() + timeout
    current = None
    while time.monotonic() < deadline:
        try:
            current = json.loads(MARKER.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(.05)
            continue
        if current.get('phase') == phase:
            return current
        time.sleep(.05)
    raise AssertionError(current)


subprocess.run([str(ROOT / 'src/tools/gnoblinctl'), 'script', 'reload'], check=True)
deadline = time.monotonic() + 5
while time.monotonic() < deadline:
    try:
        before = state()
        break
    except (AssertionError, subprocess.CalledProcessError, SyntaxError, ValueError):
        time.sleep(.05)
else:
    raise AssertionError('developer-console test object was not exported')

assert not before['exists'] and before['runDialogAbsent'] and before['lookingGlassAbsent'], before
opened = json.loads(value(call('Open')))
assert opened['opened'] and opened['open'] and opened['visible'], opened
assert opened['parent'] == 'uiGroup', opened
assert opened['transcriptRows'] == 0, opened
assert opened['y'] == opened['monitorY'], opened
assert opened['topIndex'] == opened['childCount'] - 1, opened
focused = wait_for(lambda current: current['open'] and current['focusEntry'])
assert focused['modalCount'] > 0, focused
assert opened['lookingGlassAlias'] and opened['runDialogAbsent'], opened
if shutil.which('grim'):
    time.sleep(.25)
    screenshot = Path('/tmp/gnoblin-developer-console.png')
    try:
        subprocess.run(['grim', str(screenshot)], check=True, timeout=5)
        print(f'NOTE: console frame captured at {screenshot}')
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f'NOTE: could not capture console frame: {error}')
print('PASS: Alt+F2 replacement opens a focused top-edge modal above the Shell chrome')


def evaluate(source):
    if MARKER.exists():
        MARKER.unlink()
    value(call('Evaluate', source))
    result = wait_marker('evaluated')['result']
    assert result['source'] == source, result
    assert result['error'] is None, result
    return result


first = evaluate('21 * 2')
assert first['value'] == 42, first
persistent = evaluate('let persistent = 41; persistent + 1')
assert persistent['value'] == 42, persistent
retained = evaluate('persistent += 1; persistent')
assert retained['value'] == 42, retained
awaited = evaluate('await Promise.resolve(9 * 5)')
assert awaited['value'] == 45, awaited
cleared = evaluate('console.clear(); 7')
assert cleared['value'] == 7, cleared
if MARKER.exists():
    MARKER.unlink()
value(call('Evaluate', 'throw new Error("console probe")'))
failed = wait_marker('evaluated')['result']
assert failed['error']['name'] == 'Error' and 'console probe' in failed['error']['message'], failed
print('PASS: compositor JavaScript retains bindings, supports await, and reports errors')

# Exercise the language switch through the same console evaluator entrypoint.
call('Type', 'unfinishedJavaScript')
call('Language', 'lua')
assert state()['input'] == '', state()
call('Language', 'js')
assert state()['input'] == 'unfinishedJavaScript', state()
call('Type', '')
call('Language', 'lua')
assert evaluate('21 * 2')['value'] == '42'
evaluate('counter = 40')
assert evaluate('counter + 2')['value'] == '42'
assert evaluate("local g = require('gnoblin'); g.set { shell = { probe = 7 } }; return g.config.shell.probe")['value'] == '7'
evaluate("print('lua output', counter)")
call('Type', 'math.sq')
time.sleep(.1)
lua_completion = json.loads(value(call('Complete')))
assert 'sqrt' in lua_completion['labels'], lua_completion
call('Key', 0xff09)
time.sleep(.1)
assert state()['input'] == 'math.sqrt', state()
call('Type', '')
if MARKER.exists():
    MARKER.unlink()
call('Evaluate', "error('lua probe')")
lua_error = wait_marker('evaluated')['result']
assert lua_error['error']['name'] == 'LuaError' and 'lua probe' in lua_error['error']['message'], lua_error
call('Language', 'js')
assert evaluate('persistent')['value'] == 42
call('Language', 'lua')
assert evaluate('counter')['value'] == '40'
call('Language', 'js')
print('PASS: Lua persists state, shares config helpers, prints, completes and reports errors; JavaScript state survives switching')



inspected = json.loads(value(call('Inspect')))
assert inspected['inspectorVisible'] and inspected['inspectorRows'] > 0, inspected
print('PASS: object inspection exposes descriptor rows without leaving the console')
rich = json.loads(value(call('InspectRich')))
assert rich['before'] == 0 and rich['after'] == 1, rich
assert 'getter failure' in rich['failure'], rich
assert '[[Prototype]]: ' in rich['names'] and '[Symbol(token)]: ' in rich['names'], rich
if shutil.which('grim'):
    time.sleep(.15)
    subprocess.run(['grim', '/tmp/gnoblin-js-inspector.png'], check=True)
print('PASS: getters run only on click, thrown getters stay inline, symbols and prototypes render')
call('Language', 'lua')
lua_tree = evaluate('tree = {title = "Lua inspector", items = {1, 2, 3}}; tree.self = tree; setmetatable(tree, {kind = "sample"}); return tree')
lua_expanded = json.loads(value(call('ExpandLua')))
assert '[[Metatable]]: ' in lua_expanded['names'] and '"self": ' in lua_expanded['names'], lua_expanded
if shutil.which('grim'):
    time.sleep(.15)
    subprocess.run(['grim', '/tmp/gnoblin-lua-inspector.png'], check=True)
call('Language', 'js')
print('PASS: Lua tables render expandable keys, cycles and metatables')



evaluate('console.clear(); 42')
evaluate('const name = "gnoblin"; name')
call('Type', 'global.g')
time.sleep(.1)
completion = json.loads(value(call('Complete')))
assert completion['completionVisible'] and completion['labels'] and all(
    label != '[object Object]' for label in completion['labels']), completion
print('PASS: suggestions appear while typing')
if shutil.which('grim'):
    subprocess.run(['grim', '/tmp/gnoblin-console-completion.png'], check=True)
call('Key', 0xff09)  # Tab accepts the selected suggestion.
time.sleep(.1)
assert state()['input'] == 'global.' + completion['labels'][0], state()
call('Type', 'const color = 42;')
time.sleep(.1)
assert 'foreground' in state()['attributes'], state()
if shutil.which('grim'):
    subprocess.run(['grim', '/tmp/gnoblin-console-highlight.png'], check=True)
call('Type', '')
time.sleep(.1)


value(call('Escape'))
closed = wait_for(lambda current: not current['open'] and not current['visible'])
assert not closed['focusEntry'], closed
print('PASS: Escape releases the modal grab and hides the console')


value(call('RunBinding'))
opened_again = wait_for(lambda current: current['open'])
assert opened_again['open'] and opened_again['lookingGlassAlias'], opened_again
print('PASS: the panel-run-dialog keybinding opens the developer console')
if MARKER.exists():
    MARKER.unlink()
# Let the stock NetworkManager/Polkit probes settle before changing session
# component lists. This keeps their asynchronous callbacks attached to live
# Quick Settings actors while this focused console test exercises the lock.
time.sleep(3)
value(call('StartLock'))
locked = wait_marker('locked')['state']
assert locked['locked'] and not locked['open'] and not locked['visible'], locked
restored_marker = wait_marker('restored')
assert not restored_marker['state']['locked'] and not restored_marker['state']['open'], restored_marker
print('PASS: lock-mode transition closes the console before the user session returns')


# The user-script host is recreated after unlock. Wait for the fresh test
# object and prove the replacement can be opened again without stale state.
deadline = time.monotonic() + 5
while time.monotonic() < deadline:
    try:
        reopened = json.loads(value(call('Open')))
        if reopened['open']:
            break
    except (AssertionError, SyntaxError, ValueError, subprocess.TimeoutExpired):
        time.sleep(.05)
else:
    raise AssertionError('console did not reopen after unlock')
value(call('Escape'))
print('PASS: the console is recreated cleanly after unlock')
