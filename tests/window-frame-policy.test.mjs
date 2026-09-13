import assert from "node:assert/strict";
import {
    defaults,
    validate,
    tuple,
    validateRenderers,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinFramePolicy.js";
validate(defaults);
assert.deepEqual(tuple(defaults), [0, 0, 0, 0, 0, 32, 1, 1, 1]);
assert.deepEqual(tuple({ ...defaults, mode: "replace", crop: [40, 0, 0, 0] }), [3, 40, 0, 0, 0, 32, 1, 1, 1]);
for (const invalid of [
    { mode: "ssd" },
    { crop: [1, 2] },
    { crop: [-1, 0, 0, 0] },
    { extents: [257, 0, 0, 0] },
    { background: "red; padding: 10px" },
    { "button-layout": ["close", "close"] },
    { renderer: "/tmp/theme.js" },
    { style: "" },
    { foo: 1 },
])
    assert.throws(() => validate(invalid));
validateRenderers({ cairo: ["/usr/local/bin/my-frame", "--theme=dark"] });
validateRenderers();
for (const invalid of [
    [],
    null,
    { native: ["/bin/true"] },
    { x: "shell command" },
    { x: ["relative"] },
    { x: [] },
    { x: ["/bin/true", 4] },
    { x: ["/bin/true", "\0"] },
])
    assert.throws(() => validateRenderers(invalid));
console.log("PASS frame policy validation");
