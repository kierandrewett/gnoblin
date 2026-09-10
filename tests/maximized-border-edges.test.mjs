import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function paint(frame, maximized) {
    const source = readFileSync(new URL('../src/gnome-shell-overlay/js/ui/components/gnoblinCorners.js', import.meta.url), 'utf8');
    let painted;
    const monitor = {x: -1280, y: 0, width: 1280, height: 800};
    const context = vm.createContext({
        Meta: {WindowType: {NORMAL: 0, DIALOG: 1, MODAL_DIALOG: 2}},
        Geometry: {bordersEnabled: () => true},
        windowGeometry: () => ({bounds: [0, 0, frame.width, frame.height], radius: 14, scale: 1}),
        global: {display: {get_monitor_geometry: () => monitor}},
    });
    vm.runInContext(source.slice(source.indexOf('export class WindowBorders')).replace('export class', 'class') + '\nglobalThis.Border = WindowBorders;', context);
    const border = Object.create(context.Border.prototype);
    border.actor = {meta_window: {maximized_horizontally: maximized, maximized_vertically: maximized,
        get_window_type: () => 0, is_override_redirect: () => false, get_tile_match: () => null,
        get_frame_rect: () => frame, get_monitor: () => 0}};
    border.surface = {x: 0, y: 0};
    border.widget = {set_position() {}, set_size() {}};
    border.effect = {update: g => {painted = g;}};
    border.update({'inner-width': 1, 'outer-width': 1});
    return {radius: painted.radius, bounds: [...painted.bounds]};
}
test('maximised side borders disappear while topbar and dock boundaries remain', () => {
    assert.deepEqual(paint({x: -1280, y: 32, width: 1280, height: 700}, true),
        {radius: 0, bounds: [0, 3, 1286, 703]});
});
test('all physical edges are suppressed when the work area fills the monitor', () => {
    assert.deepEqual(paint({x: -1280, y: 0, width: 1280, height: 800}, true),
        {radius: 0, bounds: [0, 0, 1286, 806]});
});
test('floating windows retain borders and rounding even against monitor edges', () => {
    assert.deepEqual(paint({x: -1280, y: 0, width: 1280, height: 800}, false),
        {radius: 14, bounds: [3, 3, 1283, 803]});
});
