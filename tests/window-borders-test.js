import * as Geometry from "../src/gnome-shell-overlay/js/ui/components/gnoblinCornerGeometry.js";
import {
    borderDefaults,
    validateBorders,
    bordersEnabled,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinCornerGeometry.js";
import { parseDocument, windowEffects } from "../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js";
function assert(ok, message) {
    if (!ok) throw new Error(message);
}
for (const value of [
    null,
    [],
    { "inner-width": -1 },
    { "outer-width": 41 },
    { "inner-width": NaN },
    { "outer-color": "black" },
    { radius: 201 },
    { typo: true },
]) {
    let rejected = false;
    try {
        validateBorders(value);
    } catch (_) {
        rejected = true;
    }
    assert(rejected, JSON.stringify(value));
}
const rules = parseDocument({
    "window-rules": [
        { match: { type: "window" }, borders: { "inner-width": 1, "inner-color": "#505050ff", "outer-width": 1 } },
        { match: { focused: true }, borders: { "outer-color": "#00000080" } },
    ],
});
const result = windowEffects({ type: "window", focused: true }, rules).borders;
assert(
    result["inner-width"] === 1 && result["inner-color"] === "#505050ff" && result["outer-color"] === "#00000080",
    "cascade",
);
assert(!bordersEnabled(borderDefaults, { normal: true }), "opt in");
assert(bordersEnabled(result, { normal: true }), "square border without corner clipping");
for (const state of ["maximized", "fullscreen", "tiled"]) {
    const expected = state !== "fullscreen";
    assert(bordersEnabled(result, { normal: true, [state]: true }) === expected, state + " default");
    assert(
        !bordersEnabled({ ...result, ["keep-" + state]: false }, { normal: true, [state]: true }),
        "disable " + state,
    );
}
print("PASS: border validation, rule cascade, fullscreen exclusion and full-size window policy");

import { validate, merge } from "../src/gnome-shell-overlay/js/ui/components/gnoblinCornerGeometry.js";
const layers = [
    { blur: 32, y: 10, opacity: 0.2 },
    { blur: 5, y: 2, opacity: 0.3 },
];
validate({ shadow: layers });
for (const shadow of [[], Array(5).fill({}), [{ blur: -1 }], [false], [{ opacity: 2 }]]) {
    let rejected = false;
    try {
        validate({ shadow });
    } catch (_) {
        rejected = true;
    }
    assert(rejected, "bad shadow stack");
}
assert(merge({ shadow: { blur: 12 } }, { shadow: layers }).shadow === layers, "array replaces table");
assert(merge({ shadow: layers }, { shadow: { blur: 8 } }).shadow.blur === 8, "table replaces array");
assert(
    merge({ shadow: { blur: 12, opacity: 0.5 } }, { shadow: { blur: 8 } }).shadow.opacity === 0.5,
    "legacy table merges",
);
print("PASS: bounded shadow layers and replacement semantics");

// Client shadows have a soft alpha ramp, followed by a sharp, stable body edge.
const bodyLine = (size, inset, alpha = 255) =>
    Array.from({ length: size }, (_, i) =>
        i < inset ? Math.round(i * 3) : i >= size - inset ? Math.round((size - 1 - i) * 3) : alpha,
    );
assert(Geometry.detectEdge(bodyLine(200, 12)) === 12, "find CSD body beyond shadow");
assert(Geometry.detectEdge(bodyLine(200, 0)) === 0, "preserve edge-to-edge window");
assert(Geometry.detectEdge(bodyLine(200, 12, 180)) === 12, "preserve translucent body");
assert(Geometry.detectEdge(Array(200).fill(0)) === null, "wait for window content");
assert(
    Geometry.detectEdge(Array.from({ length: 200 }, (_, i) => Math.min(255, i * 4))) === null,
    "do not crop a soft gradient",
);
print("PASS: client shadow edge detection");

for (const animation of [{ duration: 180, easing: "ease-out-cubic" }, { duration: 0 }, { easing: "linear" }])
    Geometry.validate({ "shadow-animation": animation });
for (const animation of [false, [], { duration: -1 }, { duration: 2001 }, { easing: "bounce" }, { speed: 2 }]) {
    let rejected = false;
    try {
        Geometry.validate({ "shadow-animation": animation });
    } catch (_) {
        rejected = true;
    }
    assert(rejected, "invalid shadow animation " + JSON.stringify(animation));
}
assert(
    Geometry.merge(
        { "shadow-animation": { duration: 200, easing: "linear" } },
        { "shadow-animation": { duration: 300 } },
    )["shadow-animation"].easing === "linear",
    "merge animation fields",
);
print("PASS: shadow fade settings");
