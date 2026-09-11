// Run with: gjs -m tests/shell-config-test.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {ConfigFile, DEFAULTS, parse, parseLegacy, minimizeTarget, Autostart, layerOffset, windowEffects} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition)
        throw new Error(message);
}

assert(JSON.stringify(parse('')) === JSON.stringify(DEFAULTS), 'missing keys use defaults');
const parsed = parseLegacy(`[protocols]
wlr-layer-shell = on
[shell]
window-switcher = yes # comment
minimize-animation = 'none' # comment
minimize-duration = 25
minimize-duration = 70
osd = off
screenshot = yes
`);
assert(parsed.osd === false && parsed.screenshot === true && parsed.notifications === null,
    'explicit feature values and unspecified persistent features');
assert(parsed['window-switcher'] && parsed['minimize-animation'] === 'none' &&
    parsed['minimize-duration'] === 70, 'comments, quotes, booleans, and last value');
assert(parseLegacy('[shell]\ninput-source-switcher = false')['input-source-switcher'] === false,
    'native input-source switcher can be disabled for external chrome');
for (const invalid of ['window-switcher = maybe', 'minimize-animation = shrink',
    'minimize-duration = -1', 'minimize-duration = 5001', 'minimize-duration = 2ms',
    'typo = true', 'notifications = maybe', '[shell']) {
    let rejected = false;
    try {
        parse(`[shell]\n${invalid}`);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject ${invalid}`);
}

const typed = parse(`[shell]
minimize-animation = "zoom"
minimize-target = [500, 900]
[[autostart]]
name = "dock"
command = ["qs", "-p", "/a path/with spaces"]
`);
assert(typed['minimize-target'][1] === 900 && typed.autostart[0].command[2] === '/a path/with spaces',
    'native TOML arrays and autostart tables');
for (const text of ['[shell]\nosd = "false"', '[shell]\nminimize-duration = 2\nminimize-duration = 3',
    '[shell]\nminimize-target = [1]', '[[autostart]]\nname = "dock"\ncommand = "qs"']) {
    let rejected = false;
    try { parse(text); } catch { rejected = true; }
    assert(rejected, `reject typed TOML error: ${text}`);
}

const dir = GLib.dir_make_tmp('gnoblin-config-test-XXXXXX');
const path = `${dir}/new/config/gnoblin.toml`;
const fragment = `${dir}/bingux.toml`;
let current;
let updates = 0;
const config = new ConfigFile(path, next => {
    current = next;
    updates++;
});
function settle() {
    const loop = new GLib.MainLoop(null, false);
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 400, () => {
        loop.quit();
        return GLib.SOURCE_REMOVE;
    });
    loop.run();
}
try {
    config.start();
    assert(current['window-switcher'] === false, 'start without file');
    GLib.file_set_contents(path, '[shell]\nwindow-switcher = true');
    settle();
    assert(current['window-switcher'] === true, 'first creation is watched');
    GLib.file_set_contents(`${path}.tmp`, '[shell]\nminimize-animation = "none"');
    Gio.File.new_for_path(`${path}.tmp`).move(Gio.File.new_for_path(path),
        Gio.FileCopyFlags.OVERWRITE, null, null);
    settle();
    assert(current['minimize-animation'] === 'none' && !current['window-switcher'],
        'atomic replacement reloads and removed keys reset');
    GLib.file_set_contents(path, '[shell]\nminimize-duration = invalid');
    settle();
    assert(current['minimize-animation'] === 'none', 'invalid edit retains last valid state');
    Gio.File.new_for_path(path).delete(null);
    settle();
    assert(current['minimize-animation'] === 'zoom', 'deletion restores defaults');
    config.destroy();
    const before = updates;
    GLib.file_set_contents(path, '[shell]\nwindow-switcher = true');
    settle();
    assert(updates === before, 'destroy cancels watcher');
    config.start();
    assert(current['window-switcher'], 'restart reads latest file');
    config.destroy();
    GLib.file_set_contents(fragment, '[shell]\nlayer-easing = "linear"\n');
    GLib.file_set_contents(path, `include = [${JSON.stringify(fragment)}]\n[shell]\nlayer-duration = 123\n`);
    const included = new ConfigFile(path, next => { current = next; updates++; });
    included.start();
    assert(current['layer-easing'] === 'linear' && current['layer-duration'] === 123,
        'included TOML merges before local settings');
    // Let the directory monitors finish subscribing before the first edit.
    settle();
    GLib.file_set_contents(fragment, '[shell]\nlayer-easing = "ease-out-quad"\n');
    settle();
    assert(current['layer-easing'] === 'ease-out-quad', 'included files are hot-reloaded');
    GLib.file_set_contents(fragment, '[[window-rules]]\nmatch.type = "window"\ncorners = { radius = 14, smoothing = 4.0 }\n');
    settle();
    assert(current['layer-easing'] === 'ease-out-quad', 'invalid included edit retains last valid settings');
    let diagnostic = '';
    try { included.reload(); } catch (error) { diagnostic = error.message; }
    assert(diagnostic.includes('expected a number from 0 to 1') && diagnostic.includes('got 4'),
        'invalid corner smoothing explains the accepted range and value');
    included.destroy();
    const target = minimizeTarget({get_icon_geometry: () => [false, null]},
        {x: 100, y: 200, width: 800, height: 600})[1];
    assert(target.x === 500 && target.y === 800, 'bottom-centre fallback');
    const icon = {x: 32, y: 64, width: 48, height: 48};
    assert(minimizeTarget({get_icon_geometry: () => [true, icon]}, null)[1] === icon,
        'dock rectangle wins');
    print('PASS: shell config parsing, watching, atomic saves, recovery, deletion, lifecycle');
} finally {
    config.destroy();
    GLib.unlink(path);
    GLib.unlink(fragment);
    GLib.rmdir(`${dir}/new/config`);
    GLib.rmdir(`${dir}/new`);
    GLib.rmdir(dir);
}

const panel = {x: -800, y: 30, width: 200, height: 40};
const monitor = {x: -800, y: 0, width: 800, height: 600};
assert(JSON.stringify(layerOffset(1 | 4 | 8, panel, monitor)) === '[0,-70]', 'top edge slides vertically');
assert(JSON.stringify(layerOffset(1 | 4, panel, monitor)) === '[-200,-70]', 'corner slides diagonally');
assert(JSON.stringify(layerOffset(2 | 8, panel, monitor)) === '[800,570]', 'bottom right respects monitor origin');
assert(JSON.stringify(layerOffset(15, panel, monitor)) === '[0,0]', 'all anchors fade without translation');
assert(parse('[shell]\nlayer-animation = "slide"\nlayer-duration = 90\nlayer-easing = "linear"')['layer-duration'] === 90, 'layer settings parse');
for (const invalid of ['layer-duration = -1', 'layer-easing = "bounce"', 'layer-animation = "zoom"']) {
    let rejected = false;
    try { parse('[shell]\n' + invalid); } catch { rejected = true; }
    assert(rejected, 'reject invalid layer option: ' + invalid);
}
print('PASS: layer animation geometry and configuration');

const rules = parse(`[[window-rules]]
match.type = "layer"
blur = 24
opacity = 0.9
[[window-rules]]
match.layer = "^dock$"
blur = 12
animation = "none"
`);
const matched = windowEffects({type: 'layer', layer: 'dock', focused: false}, rules);
assert(matched.blur === 12 && matched.opacity === 0.9 && matched.animation === 'none', 'later matching effects override individually');
assert(windowEffects({type: 'window', layer: null}, rules).blur === 0, 'layer rules do not match applications');
for (const field of ['blur = 101', 'opacity = 2', 'match.title = "["', 'match.unknown = "x"']) {
    let rejected = false;
    try { parse('[[window-rules]]\nmatch.type = "layer"\n' + field); } catch { rejected = true; }
    assert(rejected, 'reject invalid rule: ' + field);
}
print('PASS: window rule validation and precedence');

const shortcutConfig = parse(`[keybindings.shell]
show-screenshot-ui = []
[[shortcuts]]
name = "capture"
binding = "<Alt>s"
command = ["qs", "ipc", "call", "capture", "open"]
`);
assert(shortcutConfig.shortcuts[0].binding === '<Alt>s', 'TOML shortcut tables');
assert(shortcutConfig.keybindings.shell['show-screenshot-ui'].length === 0, 'TOML built-in override');
for (const text of ['[shell]\nshortcuts = []', '[[shortcuts]]\nname = "missing-fields"',
    '[keybindings.shell]\nshow-screenshot-ui = "Print"']) {
    let rejected = false;
    try { parse(text); } catch { rejected = true; }
    assert(rejected, `reject malformed shortcut TOML: ${text}`);
}
print('PASS: TOML shortcuts and keybinding groups');
