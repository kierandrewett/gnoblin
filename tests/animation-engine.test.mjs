// Exercise the shipped engine logic under Node by replacing only its GJS imports.
// Sampling, preset resolution, and preview control remain the real module code.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const sourcePath = new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinAnimation.js", import.meta.url);
let source = await readFile(sourcePath, "utf8");
source = source
    .replace(/^import Clutter from "gi:\/\/Clutter";\s*/m, "")
    .replace(/^import GLib from "gi:\/\/GLib";\s*/m, "");
const modes = {
    EASE_OUT_EXPO: 1,
    EASE_OUT_QUAD: 2,
    EASE_OUT_CUBIC: 3,
    EASE_IN_OUT_CUBIC: 4,
    LINEAR: 5,
};
globalThis.Clutter = {
    AnimationMode: modes,
    Timeline: {
        new_for_actor: (_actor, duration) => ({
            duration,
            elapsed: 0,
            handlers: {},
            set_progress_mode() {},
            connect(signal, callback) {
                this.handlers[signal] = callback;
                return signal;
            },
            set_elapsed_time(ms) {
                this.elapsed = ms;
            },
            advance(ms) {
                this.elapsed = ms;
                this.handlers["new-frame"]?.();
            },
            get_elapsed_time() {
                return this.elapsed;
            },
            start() {},
            pause() {},
            stop() {},
        }),
    },
};
const engine = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);

const actor = {
    x: 30,
    y: 40,
    width: 200,
    height: 100,
    translation_x: 0,
    translation_y: 0,
    scale_x: 1,
    scale_y: 1,
    rotation_angle: 0,
    opacity: 255,
    pivot: [0, 0],
    set_pivot_point(x, y) {
        this.pivot = [x, y];
    },
    get_pivot_point() {
        return this.pivot;
    },
};
const monitor = { x: 0, y: 0, width: 1920, height: 1080 };
const iconContext = {
    actor,
    window: { get_icon_geometry: () => [true, { x: 1800, y: 980, width: 48, height: 48 }] },
    monitor,
};

// GNOME presets keep GNOME's event durations/easing and the compositor's
// icon/monitor destination behavior. Endpoints are checked as visual values.
const minimize = engine.resolve("gnome-minimize", "minimize", iconContext);
assert.equal(minimize.duration, 400);
assert.equal(minimize.ease, "ease-out-expo");
assert.equal(minimize.target, "icon");
assert.deepEqual(engine.sample(minimize, 0), { x: 0, y: 0, scale_x: 1, scale_y: 1, opacity: 1 });
assert.deepEqual(engine.sample(minimize, 1), {
    x: 1770,
    y: 940,
    scale_x: 0.24,
    scale_y: 0.48,
    opacity: 0,
});
const gnomeOpen = engine.resolve("gnome-open", "open");
assert.equal(gnomeOpen.duration, 150);
assert.equal(gnomeOpen.ease, "ease-out-expo");
assert.equal(gnomeOpen.origin, "bottom-center");
assert.deepEqual(engine.sample(gnomeOpen, 0), { scale_x: 0.01, scale_y: 0.05, opacity: 0 });
assert.deepEqual(engine.sample(gnomeOpen, 1), { scale_x: 1, scale_y: 1, opacity: 1 });
const gnomeClose = engine.resolve("gnome-close", "close");
assert.equal(gnomeClose.duration, 150);
assert.equal(gnomeClose.ease, "ease-out-quad");
assert.deepEqual(engine.sample(gnomeClose, 0), { scale_x: 1, scale_y: 1, opacity: 1 });
assert.deepEqual(engine.sample(gnomeClose, 1), { scale_x: 0.8, scale_y: 0.8, opacity: 0 });

const layerPresets = engine.presets();
assert.ok(layerPresets.some((preset) => preset.name === "gnoblin-layer-open" && preset.event === "layer-open"));
assert.ok(!layerPresets.some((preset) => preset.name === "gnome-layer-open"));
const layer = engine.resolve("gnoblin-layer-open", "layer-open", {
    offset: [0, 72],
});
assert.equal(layer.duration, 250);
assert.equal(layer.ease, "ease-out-cubic");
assert.deepEqual(engine.sample(layer, 0), { x: 0, y: 72, opacity: 1 });
assert.deepEqual(engine.sample(layer, 1), { x: 0, y: 0, opacity: 1 });
const layerFade = engine.resolve("gnoblin-layer-open", "layer-open", { offset: [0, 0] });
assert.deepEqual(engine.sample(layerFade, 0), { x: 0, y: 0, opacity: 0 });

const custom = engine.resolve(
    "custom",
    "open",
    {},
    {
        duration: 100,
        ease: "linear",
        from: { x: 0, opacity: 0 },
        to: { x: 100, opacity: 1 },
        keyframes: [{ at: 0.5, x: 30, opacity: 0.8 }],
    },
);
assert.deepEqual(engine.sample(custom, 0.5), { x: 30, opacity: 0.8 });
assert.deepEqual(engine.sample(custom, 0.75), { x: 65, opacity: 0.9 });

// Paused preview operations use the same sampler and restore all actor state.
const previewActor = {
    ...actor,
    translation_x: 11,
    translation_y: -7,
    scale_x: 1.2,
    scale_y: 0.8,
    rotation_angle: 4,
    opacity: 123,
    pivot: [0.3, 0.6],
    set_pivot_point(x, y) {
        this.pivot = [x, y];
    },
    get_pivot_point() {
        return this.pivot;
    },
};
const preview = engine.run(previewActor, custom, { paused: true });
assert.equal(previewActor.translation_x, 0);
preview.seek(0.5);
assert.equal(previewActor.translation_x, 30);
assert.equal(previewActor.opacity, 204);
preview.step(25);
assert.equal(previewActor.translation_x, 65);
preview.cancel({ restore: true });
assert.equal(previewActor.translation_x, 11);
assert.equal(previewActor.translation_y, -7);
assert.equal(previewActor.scale_x, 1.2);
assert.equal(previewActor.scale_y, 0.8);
assert.equal(previewActor.rotation_angle, 4);
assert.equal(previewActor.opacity, 123);
assert.deepEqual(previewActor.pivot, [0.3, 0.6]);

console.log("PASS: shared animation presets, GNOME timing/endpoints, keyframe sampling, and preview controls");
