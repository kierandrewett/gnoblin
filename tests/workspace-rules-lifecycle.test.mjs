import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

function fixture(target) {
    const callbacks = new Map();
    const workspaceCallbacks = new Map();
    const moved = [];
    const context = vm.createContext({
        Config: {
            settings: { "window-rules": [] },
            windowProperties: (window) => ({ "app-id": window.appId }),
            windowEffects: () => ({
                opacity: 1,
                blur: 0,
                frame: {},
                corners: { radius: 0, mode: "off" },
                borders: { "inner-width": 0, "outer-width": 0 },
                shader: "",
                "shader-uniforms": {},
            }),
            initialWorkspaceTarget: (properties) => (properties["app-id"] === "org.example.App" ? target : null),
        },
        Workspaces: {
            resolve(selector) {
                if (selector.id === "missing") throw new Error("workspace not found");
                return selector;
            },
        },
        Meta: {
            gnoblin_window_frame_get: true,
            gnoblin_layer_namespace: (window) => window.layer ?? null,
        },
        global: {
            workspace_manager: {
                connect(name, callback) {
                    workspaceCallbacks.set(name, callback);
                    return name;
                },
                disconnect() {},
            },
            display: {
                focus_window: null,
                connect(name, callback) {
                    callbacks.set(name, callback);
                    return name;
                },
                disconnect() {},
            },
            window_manager: {
                connect(name, callback) {
                    callbacks.set(`window_manager:${name}`, callback);
                    return name;
                },
                disconnect() {},
            },
            get_window_actors: () => [],
        },
        BackdropRedraw: class {
            set() {}
        },
        BackgroundEffects: class {
            refresh() {}
            owns() {
                return false;
            }
        },
        ToolkitCache: class {},
        GLib: { SOURCE_REMOVE: false },
    });
    const source = readFileSync(
        new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinRules.js", import.meta.url),
        "utf8",
    );
    vm.runInContext(
        source.slice(source.indexOf("export class WindowRules")).replace("export class", "globalThis.Rules = class"),
        context,
    );
    return { owner: new context.Rules(), callbacks, workspaceCallbacks, moved, context };
}

function window(appId, transient = null, layer = null) {
    return {
        appId,
        layer,
        workspace: { id: "current" },
        get_transient_for: () => transient,
        connect: () => 1,
        disconnect() {},
        change_workspace(workspace) {
            this.workspace = workspace;
        },
        get_workspace() {
            return this.workspace;
        },
    };
}

test("initial placement moves a matching top level window once", () => {
    const f = fixture({ id: "code" });
    const target = window("org.example.App");
    f.owner._assignInitialWorkspace(target);
    assert.deepEqual(target.workspace, { id: "code" });
    f.owner._assignInitialWorkspace(target);
    assert.deepEqual(target.workspace, { id: "code" });
});

test("initial placement waits for map and runs before visual rule application", () => {
    const f = fixture({ id: "code" });
    const target = window("org.example.App");
    const actor = { meta_window: target };
    const appliedOn = [];
    f.owner._apply = (mappedActor) => appliedOn.push(mappedActor.meta_window.workspace);

    f.callbacks.get("window_manager:map")(null, actor);

    assert.deepEqual(target.workspace, { id: "code" });
    assert.deepEqual(appliedOn, [{ id: "code" }]);
});

test("initial placement leaves transients and nonmatching windows with their parent or current workspace", () => {
    const f = fixture({ id: "code" });
    const parent = window("parent");
    const dialog = window("org.example.App", parent);
    const other = window("org.example.Other");
    f.owner._assignInitialWorkspace(dialog);
    f.owner._assignInitialWorkspace(other);
    assert.deepEqual(dialog.workspace, { id: "current" });
    assert.deepEqual(other.workspace, { id: "current" });
});

test("initial placement skips layer surfaces", () => {
    const f = fixture({ id: "code" });
    const layer = window("org.example.App", null, "panel");
    f.owner._assignInitialWorkspace(layer);
    assert.deepEqual(layer.workspace, { id: "current" });
});

test("unavailable placement target leaves the new window in place", () => {
    const f = fixture({ id: "missing" });
    const target = window("org.example.App");
    const original = console.warn;
    console.warn = () => {};
    try {
        f.owner._assignInitialWorkspace(target);
    } finally {
        console.warn = original;
    }
    assert.deepEqual(target.workspace, { id: "current" });
});

test("a window without a workspace is left eligible until a workspace exists", () => {
    const f = fixture({ id: "code" });
    const target = window("org.example.App");
    target.workspace = null;
    f.owner._assignInitialWorkspace(target);
    target.workspace = { id: "current" };
    f.owner._assignInitialWorkspace(target);
    assert.deepEqual(target.workspace, { id: "code" });
});

test("workspace number rules refresh only windows whose position changed", () => {
    const f = fixture(null);
    f.owner._hasWorkspaceNumberRules = true;
    const firstWorkspace = { index: () => 0 };
    const secondWorkspace = { index: () => 1 };
    const unchangedActor = { meta_window: { get_workspace: () => firstWorkspace } };
    const changedActor = { meta_window: { get_workspace: () => secondWorkspace } };
    const scheduled = [];
    f.owner._actors.set(unchangedActor, { workspaceNumber: 1 });
    f.owner._actors.set(changedActor, { workspaceNumber: 1 });
    f.owner._schedule = (actor) => scheduled.push(actor);

    f.workspaceCallbacks.get("workspaces-reordered")();

    assert.deepEqual(scheduled, [changedActor]);
    assert.equal(f.owner._actors.get(changedActor).workspaceNumber, 2);
});

test("first application caches the MetaWorkspace index, not the change signal ID", () => {
    const f = fixture(null);
    f.context.Meta.gnoblin_window_frame_get = null;
    const workspace = { index: () => 1 };
    const metaWindow = {
        get_workspace: () => workspace,
        connect: () => 1,
        disconnect() {},
    };
    const surface = {
        opacity: 255,
        get_name: () => "client-surface",
        connect: () => 1,
    };
    const actor = {
        meta_window: metaWindow,
        get_children: () => [surface],
        connect: () => 1,
        disconnect() {},
        add_effect_with_name() {},
    };

    f.owner._apply(actor);

    assert.equal(f.owner._actors.get(actor).workspaceNumber, 2);
});

test("workspace ID rules do not refresh when numbering alone changes", () => {
    const f = fixture(null);
    const workspace = { index: () => 1 };
    const actor = { meta_window: { get_workspace: () => workspace } };
    let scheduled = false;
    f.owner._actors.set(actor, { workspaceNumber: 1 });
    f.owner._schedule = () => {
        scheduled = true;
    };

    f.workspaceCallbacks.get("workspace-removed")();

    assert.equal(scheduled, false);
});

test("workspace removal refreshes windows whose current number shifted", () => {
    const f = fixture(null);
    f.owner._hasWorkspaceNumberRules = true;
    const workspace = { index: () => 1 };
    const actor = { meta_window: { get_workspace: () => workspace } };
    const scheduled = [];
    f.owner._actors.set(actor, { workspaceNumber: 2 });
    f.owner._schedule = (candidate) => scheduled.push(candidate);

    workspace.index = () => 0;
    f.workspaceCallbacks.get("workspace-removed")();

    assert.deepEqual(scheduled, [actor]);
    assert.equal(f.owner._actors.get(actor).workspaceNumber, 1);
});
