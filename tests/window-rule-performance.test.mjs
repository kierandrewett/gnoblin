import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

function loadConfig(CountingRegExp) {
    const path = new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js", import.meta.url);
    const source = readFileSync(path, "utf8")
        .replace(/^import .*;\n/gm, "")
        .replace(/^export /gm, "");
    const context = vm.createContext({
        RegExp: CountingRegExp,
        Permissions: { DEFAULT_POLICY: {}, validate: (value) => value ?? {} },
        Corners: {
            borderDefaults: {},
            defaults: {},
            merge: (previous, next) => ({ ...previous, ...next }),
            validate() {},
            validateBorders() {},
        },
        Gio: {
            SettingsSchemaSource: {
                get_default() {
                    return {
                        lookup() {
                            return null;
                        },
                    };
                },
            },
        },
        GLib: {},
        Meta: {},
        Clutter: {},
    });
    vm.runInContext(
        `${source}\nglobalThis.windowEffects = windowEffects; globalThis.parseDocument = parseDocument;`,
        context,
    );
    return { windowEffects: context.windowEffects, parseDocument: context.parseDocument };
}

test("window rule regular expressions compile once and survive configuration replacement", () => {
    const NativeRegExp = RegExp;
    let constructions = 0;
    function CountingRegExp(...args) {
        constructions++;
        return new NativeRegExp(...args);
    }
    const { windowEffects, parseDocument } = loadConfig(CountingRegExp);
    const rules = Array.from({ length: 6 }, (_, index) => ({ match: { title: `^window-${index}$` }, opacity: 0.9 }));
    const config = parseDocument({ "window-rules": rules });
    assert.equal(constructions, 6, "six validated expressions compile during configuration installation");

    // Before caching, this workload constructed 60,000 regular expressions.
    const start = performance.now();
    for (let index = 0; index < 10_000; index++)
        windowEffects({ type: "window", focused: false, title: "window-5", layer: null, "app-id": "" }, config);
    const elapsed = performance.now() - start;
    assert.equal(constructions, 6, "10,000 evaluations compile no further regular expressions");

    rules[5].match = { title: "^replacement$" };
    windowEffects({ type: "window", focused: false, title: "replacement", layer: null, "app-id": "" }, config);
    assert.equal(constructions, 7, "a replacement match object invalidates its compiled matcher");

    const replacement = parseDocument({ "window-rules": [{ match: { title: "^new-config$" } }] });
    windowEffects({ type: "window", focused: false, title: "new-config", layer: null, "app-id": "" }, replacement);
    assert.equal(constructions, 8, "a replacement configuration compiles only its new matcher");
    assert.throws(
        () => parseDocument({ "window-rules": [{ match: { title: "[" } }] }),
        "configuration installation still rejects invalid regular expressions",
    );
    assert.ok(Number.isFinite(elapsed) && elapsed >= 0, "the real matcher path was timed");
});
