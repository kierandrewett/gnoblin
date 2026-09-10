import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function fixture(rules) {
    const callbacks = new Map();
    const idle = [];
    const actors = Array.from({length: 20}, (_, id) => ({id}));
    const display = {focus_window: {get_compositor_private: () => actors[0]},
        connect(name, callback) { callbacks.set(name, callback); return 1; }};
    const context = vm.createContext({
        Config: {settings: {'window-rules': rules}},
        global: {display, window_manager: {connect() {return 1;}}},
        BackdropRedraw: class {}, ToolkitCache: class {},
        GLib: {idle_add(priority, callback) {idle.push(callback); return idle.length;}, SOURCE_REMOVE: false},
    });
    const source = readFileSync(new URL('../src/gnome-shell-overlay/js/ui/components/gnoblinRules.js', import.meta.url), 'utf8');
    vm.runInContext(source.slice(source.indexOf('export class WindowRules')).replace('export class', 'globalThis.Rules = class'), context);
    const owner = new context.Rules();
    for (const actor of actors) owner._actors.set(actor, {});
    const applied = [];
    owner._apply = actor => applied.push(actor.id);
    return {owner, actors, applied, flush() {while (idle.length) idle.shift()();},
        focus(index) {
            display.focus_window = index === null ? null : {get_compositor_private: () => actors[index]};
            callbacks.get('notify::focus-window')();
        }};
}

test('focus-dependent rules refresh only previous and current windows', () => {
    const f = fixture([{match: {focused: true}}]);
    f.focus(5); f.flush();
    assert.deepEqual(f.applied, [0, 5]);
    f.applied.length = 0;
    f.focus(null); f.flush();
    assert.deepEqual(f.applied, [5]);
});

test('focus-independent rules require no focus refresh', () => {
    const f = fixture([{match: {type: 'window'}}]);
    f.focus(5); f.flush();
    assert.deepEqual(f.applied, []);
});

test('duplicate geometry events coalesce and destroyed windows are skipped', () => {
    const f = fixture([]);
    for (let i = 0; i < 100; i++) f.owner._schedule(f.actors[1]);
    f.owner._schedule(f.actors[2]);
    f.owner._actors.delete(f.actors[2]);
    f.flush();
    assert.deepEqual(f.applied, [1]);
});
