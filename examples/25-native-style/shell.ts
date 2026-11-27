// Example 25 in TypeScript: the styles and transitions of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Gives every window rounded corners and two shadows, stages a transition that also changes the opacity of every window and a node of
// its own, and aborts it: the library puts back the opacity, and the shell's rollback is told before `abort` answers. Every line starts
// with `style:`, as the Rust shell of the same example spells it.
import {
    BlurFilter,
    Border,
    BrightnessFilter,
    ClientStyle,
    ContrastFilter,
    SaturateFilter,
    SceneNode,
    ShaderFilter,
    Shadow,
    Transition,
} from "gnoblin-std";
import type { Filter } from "gnoblin-std";
import type {} from "gnoblin-std";

const style = ClientStyle.ofClients();
style.cornerRadius = { all: 12 };
style.shadows = [{ blur: 8 }, { y: 4, blur: 20 }];
const radius = style.cornerRadius.all;
const shadows = style.shadows;
console.log(`style: corner radius ${radius}, ${shadows.length} shadows, the first blur ${shadows[0].blur}`);

// The value classes: every one is built with its defaults and with values of its own, and the library hands back what it was given.
const SHADER = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }";
style.borders = [new Border(), new Border({ width: 3, color: "#ff0000", offset: -2, inset: true })];
style.shadows = [new Shadow(), new Shadow({ x: 2, y: 4, blur: 20, spread: 3, color: "#112233", inset: true })];
style.backdropFilter = [
    new BlurFilter(),
    new BlurFilter({ radius: 20 }),
    new SaturateFilter(),
    new SaturateFilter({ amount: 1.5 }),
    new BrightnessFilter(),
    new BrightnessFilter({ amount: 0.5 }),
    new ContrastFilter(),
    new ContrastFilter({ amount: 1.25 }),
];
style.filter = [new ShaderFilter({ source: SHADER }), new ShaderFilter({ source: SHADER, params: [0.5, 2] })];
const describe = (filter: Filter): string => {
    const value =
        "radius" in filter ? filter.radius : "amount" in filter ? filter.amount : filter.params.join(",") || "-";
    return `${filter.constructor.name.replace("Filter", "").toLowerCase()} ${value}`;
};
console.log(
    `style: value classes borders ${style.borders.map((b) => `${b.width} ${b.color} ${b.offset} ${b.inset}`).join(" | ")}`,
);
console.log(
    `style: value classes shadows ${style.shadows.map((s) => `${s.x} ${s.y} ${s.blur} ${s.spread} ${s.color} ${s.inset}`).join(" | ")}`,
);
console.log(`style: value classes backdrop filters ${style.backdropFilter.map(describe).join(" | ")}`);
console.log(`style: value classes filters ${style.filter.map(describe).join(" | ")}`);

// A bare number is the radius of every corner, and a filter may be written as an object with one key, its kind.
style.cornerRadius = 12;
style.backdropFilter = [{ blur: 24 }, { saturate: 1.35 }];
style.filter = [{ shader: { source: SHADER, params: [2] } }];
const backdrop = style.backdropFilter.map(describe).join(" | ");
const filters = style.filter.map(describe).join(" | ");
console.log(`style: shorthands corner ${style.cornerRadius?.all}, backdrop ${backdrop}, filter ${filters}`);

const probe = new SceneNode({ id: "probe" });
let rolled = 0;
Transition.begin({ duration: 200 }, () => {
    rolled += 1;
});
Transition.set(probe.node, "x", { number: 40 });
style.opacity = 0.5;
Transition.abort();
console.log(
    `style: transition aborted, rollback ran ${rolled}, opacity is set ${style.isSet("opacity")}, x is ${probe.x}`,
);
