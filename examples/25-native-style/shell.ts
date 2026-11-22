// Example 25 in TypeScript: the styles and transitions of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Gives every window rounded corners and two shadows, stages a transition that also changes the opacity of every window and a node of
// its own, and aborts it: the library puts back the opacity, and the shell's rollback is told before `abort` answers. Every line starts
// with `style:`, as the Rust shell of the same example spells it.
import { ClientStyle, SceneNode, Transition } from "gnoblin-std/generated/objects";
import type {} from "gnoblin-std";

const style = ClientStyle.ofClients();
style.cornerRadius = { all: 12 };
style.shadows = [{ blur: 8 }, { y: 4, blur: 20 }];
const radius = style.cornerRadius.all;
const shadows = style.shadows;
console.log(`style: corner radius ${radius}, ${shadows.length} shadows, the first blur ${shadows[0].blur}`);

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
