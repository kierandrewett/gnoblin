import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

function fixture() {
    const source = readFileSync(
        new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinCorners.js", import.meta.url),
        "utf8",
    );
    const start = source.indexOf("const GeometryEffect =");
    const end = source.indexOf("\nconst CornersEffect =", start);
    const context = vm.createContext({
        GObject: { registerClass: (value) => value },
        Shell: {
            GLSLEffect: class {
                _init() {}
                vfunc_paint_target() {}
                set_geometry_uniforms() {}
            },
        },
    });
    vm.runInContext(source.slice(start, end) + "\nglobalThis.Effect = GeometryEffect;", context);
    const effect = new context.Effect();
    effect._init();
    const values = new Map();
    let requests = 0;
    effect.set_geometry_uniforms = (width, height) => values.set("nativeGeometry", [width, height]);
    effect.get_uniform_location = (name) => name;
    effect.set_uniform_float = (name, size, data) => values.set(name, Array.from(data));
    effect.queue_repaint = () => {};
    const uniform = effect.uniform.bind(effect);
    effect.uniform = (...args) => {
        requests++;
        uniform(...args);
    };
    return { effect, values, requests: () => requests };
}

test("painting does not enter a JavaScript effect override", () => {
    const f = fixture();
    for (let i = 0; i < 1000; i++) f.effect.vfunc_paint_target();
    assert.equal(f.requests(), 0);
    assert.equal(Object.hasOwn(Object.getPrototypeOf(f.effect), "vfunc_paint_target"), false);
});

test("fractional logical dimensions reach the native geometry updater", () => {
    const f = fixture();
    f.effect.uniform("dimensions", [320.4, 240.4]);
    assert.deepEqual(f.values.get("nativeGeometry"), [320.4, 240.4]);
});

test("uniform cache retains its own copy when a caller changes an array", () => {
    const f = fixture();
    const bounds = [1, 2, 3, 4];
    f.effect.uniform("bounds", bounds);
    bounds[0] = 9;
    f.effect.uniform("bounds", bounds);
    assert.deepEqual(f.values.get("bounds"), [9, 2, 3, 4]);
});
