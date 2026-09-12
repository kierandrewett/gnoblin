import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../src/scripts/lib/layer-companions.js", import.meta.url), "utf8")
    .replace(/^import .*;\n/gm, "")
    .replace("export class LayerCompanions", "this.LayerCompanions = class LayerCompanions");
const signals = {
    connect() {
        return 1;
    },
    disconnect() {},
};
const parent = {
    children: [],
    get_children() {
        return this.children.slice();
    },
    place(child, sibling, above) {
        this.children = this.children.filter((value) => value !== child);
        const index = sibling ? this.children.indexOf(sibling) + (above ? 1 : 0) : 0;
        this.children.splice(index, 0, child);
    },
    set_child_above_sibling(child, sibling) {
        this.place(child, sibling, true);
    },
    set_child_below_sibling(child, sibling) {
        this.place(child, sibling, false);
    },
};
const actor = (name) => ({
    ...signals,
    visible: true,
    meta_window: {
        name,
        get_monitor() {
            return 0;
        },
    },
    get_parent() {
        return parent;
    },
});
const bar = actor("bar"),
    dock = actor("dock"),
    app = actor("fullscreen"),
    editor = actor("editor"),
    popup = actor("popup");
const original = [bar, dock, app, editor, popup];
parent.children = original.slice();
const context = vm.createContext({
    global: { display: signals, window_manager: signals, get_window_actors: () => original },
    Main: { sessionMode: { ...signals, isLocked: false } },
    Meta: { gnoblin_layer_namespace: (window) => window.name },
});
vm.runInContext(source, context);
const layers = new context.LayerCompanions();
layers.update([{ surface: "editor", companions: ["dock", "bar"], companionsAbove: true }]);
assert.deepEqual(
    parent.children,
    [app, editor, bar, dock, popup],
    "Raise actual containers above the editor in their existing order",
);
layers.apply();
assert.deepEqual(parent.children, [app, editor, bar, dock, popup], "Repeated application preserves order");
layers.update([]);
assert.deepEqual(parent.children, original, "Closing the editor restores the exact original stack");
layers.update([{ surface: "popup", companions: ["bar", "dock"] }]);
assert.deepEqual(
    parent.children,
    [app, editor, bar, dock, popup],
    "Existing search companions remain below their overlay",
);
context.Main.sessionMode.isLocked = true;
layers.apply();
assert.deepEqual(parent.children, original, "The lock screen cancels temporary stacking");
layers.destroy();
let namespaceCalls = 0;
const performanceContext = vm.createContext({
    global: { display: signals, window_manager: signals, get_window_actors: () => original },
    Main: { sessionMode: { ...signals, isLocked: false } },
    Meta: {
        gnoblin_layer_namespace(window) {
            namespaceCalls++;
            return window.name;
        },
    },
});
vm.runInContext(source, performanceContext);
const performanceLayers = new performanceContext.LayerCompanions();
performanceLayers.apply();
assert.equal(namespaceCalls, 0, "No companion request does not inspect layer namespaces");
performanceLayers.update([
    { surface: "editor", companions: ["bar", "dock"] },
    { surface: "popup", companions: ["bar", "dock"] },
]);
assert.equal(
    namespaceCalls,
    original.length,
    "One apply reads each visible layer namespace once, regardless of request count",
);
performanceLayers.destroy();
const sessionSource = readFileSync(new URL("../src/scripts/lib/ui-sessions.js", import.meta.url), "utf8");
const { UiSessions } = await import("data:text/javascript;base64," + Buffer.from(sessionSource).toString("base64"));
let request;
const sessions = new UiSessions(() => {}, {
    update(value) {
        request = value;
    },
});
sessions.command(
    {},
    {
        action: "state",
        name: "customise",
        state: {
            visible: true,
            surface: "editor",
            companions: ["bar", "dock"],
            companionsAbove: true,
        },
    },
);
assert.deepEqual(
    request,
    [{ surface: "editor", companions: ["bar", "dock"], companionsAbove: true }],
    "The compositor session preserves the requested direction",
);
console.log("PASS: editor container order, repeated application, restoration, existing popouts and lock state");
