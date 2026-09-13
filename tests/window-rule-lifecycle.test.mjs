import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const directory = new URL("../src/gnome-shell-overlay/js/ui/components/", import.meta.url);
test("late script cleanup cannot recreate a destroyed window-rule owner", () => {
    let scans = 0;
    const source = readFileSync(new URL("gnoblinRules.js", directory), "utf8");
    const context = vm.createContext({
        Meta: {},
        Config: { settings: { "window-rules": [] } },
        global: {
            display: { connect() {}, disconnect() {} },
            window_manager: { connect() {}, disconnect() {} },
            get_window_actors() {
                scans++;
                return [];
            },
        },
        BackdropRedraw: class {
            destroy() {}
        },
        ToolkitCache: class {
            destroy() {}
        },
        BackgroundEffects: class {
            destroy() {}
            refresh() {}
        },
    });
    vm.runInContext(
        source.slice(source.indexOf("export class WindowRules")).replace("export class", "globalThis.Rules = class"),
        context,
    );
    const rules = new context.Rules();
    rules.destroy();
    // A script disposer may still hold the old rules across lock/unlock.
    rules.refresh();
    assert.equal(scans, 0, "destroyed owner scanned windows again");
    assert.doesNotThrow(() => rules._apply(null), "late callbacks must stop before touching actors");
    assert.doesNotThrow(() => rules.destroy(), "teardown must be idempotent");
});

test("controller unloads scripts before destroying the services they use", () => {
    const source = readFileSync(new URL("gnoblinControl.js", directory), "utf8");
    const start = source.indexOf("    disable() {");
    const end = source.indexOf("\n    // --- desktop state", start);
    const disable = source.slice(start, end);
    const script = disable.indexOf("this._scripts.destroy()");
    for (const service of ["this._windowRules?.destroy()", "this._config?.destroy()", "this._bus.destroy()"])
        assert.ok(script >= 0 && script < disable.indexOf(service), `${service} ran before script cleanup`);
});
