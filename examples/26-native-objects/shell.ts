// Example 26 in TypeScript: workspaces and layouts of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Makes a workspace with the library's master and stack layout. When the third window joins, the workspace gets a layout this shell
// implements (equal columns); when the fourth joins it goes back to the library's layout and the shell moves the divider as a drag would
// (`findSplit`, `setSplit`). Every line starts with `objects:`, as the Rust shell of the same example spells it.
import { Gnoblin } from "gnoblin-core/generated/objects";
import { Desks, Layout, MasterStackLayout, Workspace, Workspaces } from "gnoblin-std/generated/objects";
import type { LayoutContextInit, LayoutPlacementInit } from "gnoblin-std/generated/objects";
import type {} from "gnoblin-std";

class Columns extends Layout {
    compute(context: LayoutContextInit): LayoutPlacementInit[] {
        const area = context.area;
        const gap = context.outerGap ?? 0;
        const width = Math.floor((area.width - 2 * gap) / Math.max(context.children.length, 1));
        return context.children.map((node, at) => ({
            node,
            bounds: { x: area.x + gap + width * at, y: area.y + gap, width, height: area.height - 2 * gap },
            tiled: true,
        }));
    }
}

const main = new Workspace("main");
main.gaps = { inner: 6, outer: 12 };
main.layout = new MasterStackLayout();
const workspaces = Workspaces.get();
workspaces.add(main);
console.log(`objects: the workspace ${main.id} is in the list at index ${main.index}`);

let phase = 0;
const clients = main.clients!;
const desks = Desks.get();
clients.on("added", (node: number) => {
    const count = clients.length;
    console.log(`objects: client ${node} joined main, ${count} windows`);
    if (count >= 3 && phase === 0) {
        phase = 1;
        main.layout = new Columns();
        console.log(`objects: a layout of the shell's own tiles ${count} windows: true`);
    } else if (count >= 4 && phase === 1) {
        phase = 2;
        main.layout = new MasterStackLayout();
        console.log(`objects: back to the library's layout with ${count} windows: true`);
    } else if (count >= 5 && phase === 2) {
        // The library has laid the four windows out by now, so the divider is there to be found.
        phase = 3;
        // The divider is asked of a window: the `Client` of the first member, which the core looks up by its id.
        const area = desks.areas().area;
        const id = clients.members[0];
        const first = id === undefined ? undefined : Gnoblin.get().clients.get(id);
        if (area === undefined || first === undefined) {
            console.log("objects: cannot find the divider");
            return;
        }
        for (let x = area.x; x < area.x + area.width; x += 25) {
            const split = main.findSplit(first, x, area.y + area.height / 2);
            if (split !== undefined) {
                const to = Math.min(Math.max(0.7, split.min), split.max);
                main.setSplit(split.token, to);
                console.log(`objects: the divider is at ${split.ratio} and moves to ${to}: true`);
                return;
            }
        }
        console.log("objects: no divider found");
    }
});
console.log("objects: running");
