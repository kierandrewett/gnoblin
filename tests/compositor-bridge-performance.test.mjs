import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/scripts/lib/window-switcher-fallback.js', import.meta.url), 'utf8')
    .replace(/^import .*;\n/gm, '').replace('export class WindowSwitcherFallback', 'class WindowSwitcherFallback');
const windows = Array.from({length: 200}, (_, index) => ({index}));
let liveScans = 0;
const membership = {count: 0};
class CountingSet extends Set {
    has(value) { membership.count++; return super.has(value); }
}
const context = vm.createContext({
    Meta: {KeyBindingAction: {NONE: 0}},
    Clutter: {ModifierType: {}, KEY_Escape: 1, KEY_Return: 2, KEY_KP_Enter: 3,
        KEY_Left: 4, KEY_Up: 5, KEY_Right: 6, KEY_Down: 7},
    GLib: {}, Main: {sessionMode: {isLocked: false}}, Set: CountingSet,
    global: {display: {list_all_windows() { liveScans++; return windows; }}, get_current_time: () => 0},
});
vm.runInContext(source + '\nglobalThis.WindowSwitcherFallback = WindowSwitcherFallback;', context);
const fallback = Object.create(context.WindowSwitcherFallback.prototype);
fallback.gesture = {windows: windows.slice(), selected: 0};
fallback.move(1);
assert.equal(liveScans, 1, 'One live-window enumeration serves one switcher step');
assert.equal(fallback.gesture.selected, 1);

fallback.move(1);
assert.equal(liveScans, 2, 'A subsequent step performs one fresh enumeration');
assert.equal(membership.count, windows.length * 2, 'Filtering has linear live-window membership checks');
console.log('PASS: switcher navigation uses one linear live-window membership pass');
