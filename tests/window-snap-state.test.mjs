import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

function fixture(flags = 0) {
    const calls = [];
    const source = readFileSync(new URL("../src/scripts/lib/window-snap.js", import.meta.url), "utf8")
        .replace(/^import .*;\n/gm, "")
        .replace("export class WindowSnap", "class WindowSnap");
    const area = { x: 0, y: 32, width: 1280, height: 768 };
    const context = vm.createContext({
        Meta: { MaximizeFlags: { BOTH: 3 } },
        Main: { sessionMode: { isLocked: false }, layoutManager: { monitors: [{ index: 0 }] }, activateWindow() {} },
        global: { get_current_time: () => 1 },
    });
    vm.runInContext(source + "\nglobalThis.WindowSnap = WindowSnap;", context);
    const snap = Object.create(context.WindowSnap.prototype);
    snap.saved = new Map();
    snap.bridge = { eligible: () => true };
    const window = {
        get_stable_sequence: () => 1,
        get_maximize_flags: () => flags,
        unmaximize: () => {
            flags = 0;
            calls.push("restore");
        },
        maximize: () => {
            flags = 3;
            calls.push("maximize");
        },
        can_minimize: () => true,
        can_maximize: () => true,
        minimize: () => calls.push("minimize"),
        allows_move: () => true,
        allows_resize: () => true,
        is_fullscreen: () => false,
        get_workspace: () => ({ get_work_area_for_monitor: () => area }),
        move_to_monitor: () => calls.push("monitor"),
        move_resize_frame: (...args) => calls.push(args),
    };
    return { snap, window, calls, area };
}
test("maximised and native tiled windows restore before minimising", () => {
    for (const flags of [1, 2, 3]) {
        const { snap, window, calls } = fixture(flags);
        snap.restoreOrMinimize(window);
        snap.restoreOrMinimize(window);
        assert.deepEqual(calls, ["restore", "minimize"]);
    }
});
test("custom snapped windows restore their saved rectangle once", () => {
    const { snap, window, calls } = fixture();
    snap.saved.set("1", { x: 30, y: 50, width: 640, height: 400 });
    snap.restoreOrMinimize(window);
    snap.restoreOrMinimize(window);
    assert.deepEqual(calls, [[true, 30, 50, 640, 400], "minimize"]);
});
test("top-edge snap uses native maximisation instead of a frame resize", () => {
    const { snap, window, calls, area } = fixture();
    snap.saved.set("1", {});
    snap.apply(window, area, 0, null, true);
    assert.deepEqual(calls, ["monitor", "maximize"]);
    assert.equal(snap.saved.size, 0);
});
