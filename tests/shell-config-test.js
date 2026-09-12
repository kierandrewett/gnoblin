// Lua evaluation belongs to Mutter. This suite checks the Shell schema that
// receives its resulting document.
import {DEFAULTS, parseDocument, minimizeTarget, layerOffset, windowEffects} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition)
        throw new Error(message);
}

assert(JSON.stringify(parseDocument({})) === JSON.stringify(DEFAULTS), 'missing Lua keys use defaults');
const typed = parseDocument({shell: {'minimize-animation': 'zoom', 'minimize-target': [500, 900]}, autostart: [{name: 'dock', command: ['qs', '-p', '/a path/with spaces']}]});
assert(typed['minimize-target'][1] === 900 && typed.autostart[0].command[2] === '/a path/with spaces', 'Lua arrays and autostart records retain values');
for (const document of [{shell: {osd: 'false'}}, {shell: {'minimize-duration': 5001}}, {shell: {'minimize-target': [1]}}, {autostart: [{name: 'dock', command: 'qs'}]}]) {
    let rejected = false;
    try { parseDocument(document); } catch { rejected = true; }
    assert(rejected, `reject invalid Lua document: ${JSON.stringify(document)}`);
}

const target = minimizeTarget({get_icon_geometry: () => [false, null]}, {x: 100, y: 200, width: 800, height: 600})[1];
assert(target.x === 500 && target.y === 800, 'bottom-centre fallback');
const icon = {x: 32, y: 64, width: 48, height: 48};
assert(minimizeTarget({get_icon_geometry: () => [true, icon]}, null)[1] === icon, 'dock rectangle wins');
const panel = {x: -800, y: 30, width: 200, height: 40};
const monitor = {x: -800, y: 0, width: 800, height: 600};
assert(JSON.stringify(layerOffset(1 | 4 | 8, panel, monitor)) === '[0,-70]', 'top edge slides vertically');
assert(JSON.stringify(layerOffset(1 | 4, panel, monitor)) === '[-200,-70]', 'corner slides diagonally');
assert(JSON.stringify(layerOffset(2 | 8, panel, monitor)) === '[800,570]', 'bottom right respects monitor origin');

const rules = parseDocument({'window-rules': [{match: {type: 'layer'}, blur: 24, opacity: 0.9}, {match: {layer: '^dock$'}, blur: 12, animation: 'none'}]});
const matched = windowEffects({type: 'layer', layer: 'dock', focused: false}, rules);
assert(matched.blur === 12 && matched.opacity === 0.9 && matched.animation === 'none', 'later Lua rules override individual effects');
assert(windowEffects({type: 'window', layer: null}, rules).blur === 0, 'layer rules do not match applications');
for (const document of [{'window-rules': [{match: {type: 'layer'}, blur: 101}]}, {'window-rules': [{match: {type: 'layer'}, opacity: 2}]}, {'window-rules': [{match: {title: '['}}]}, {'window-rules': [{match: {unknown: 'x'}}]}]) {
    let rejected = false;
    try { parseDocument(document); } catch { rejected = true; }
    assert(rejected, `reject invalid window rule: ${JSON.stringify(document)}`);
}

const shortcuts = parseDocument({keybindings: {shell: {'show-screenshot-ui': []}}, shortcuts: [{name: 'capture', binding: '<Alt>s', command: ['qs', 'ipc', 'call', 'capture', 'open']}]});
assert(shortcuts.shortcuts[0].binding === '<Alt>s', 'Lua shortcut record accepted');
assert(shortcuts.keybindings.shell['show-screenshot-ui'].length === 0, 'Lua built-in override accepted');
for (const document of [{shell: {shortcuts: []}}, {shortcuts: [{name: 'missing-fields'}]}, {keybindings: {shell: {'show-screenshot-ui': 'Print'}}}]) {
    let rejected = false;
    try { parseDocument(document); } catch { rejected = true; }
    assert(rejected, `reject malformed Lua shortcut document: ${JSON.stringify(document)}`);
}
print('PASS: Lua shell configuration schema, rules, shortcuts, and geometry');
