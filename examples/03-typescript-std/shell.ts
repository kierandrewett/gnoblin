// Example 3 in TypeScript: a shell on the classes generated from the catalog of `libgnoblin-std`.
//
//     gnoblin ./shell.ts
//
// The same shell as examples 3 (Python) and 6 (Rust), in TypeScript:
//
// - `Columns` is a `Layout`: given the clients of a workspace and its area, it answers with where each goes, one column each, the
//   newest on the right, each edge floored to a whole pixel on its own. The library calls it whenever the workspace changes and
//   writes what it answers;
// - `new Workspace` makes the one workspace, with a gap of 10 between tiles and 20 to the screen's edge, and `Workspaces` lists it;
// - a listener on the collection of clients says where each window opens;
// - `Super+Tab` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.
import { Gnoblin } from "gnoblin-core/generated/objects";
import { Layout, Workspace, Workspaces } from "gnoblin-std/generated/objects";
import type { LayoutContextInit, LayoutPlacementInit } from "gnoblin-std/generated/objects";

/** Equal-width columns across the workspace, separated by the workspace's inner gap. */
class Columns extends Layout {
    compute(context: LayoutContextInit): LayoutPlacementInit[] {
        const count = context.children.length;
        if (count === 0) {
            return [];
        }
        const { innerGap = 0, outerGap = 0 } = context;
        const left = (context.area.x ?? 0) + outerGap;
        const top = Math.floor((context.area.y ?? 0) + outerGap);
        const bottom = Math.floor((context.area.y ?? 0) + (context.area.height ?? 0) - outerGap);
        const width = Math.max(((context.area.width ?? 0) - 2 * outerGap - innerGap * (count - 1)) / count, 0);
        return context.children.map((node, index) => {
            const start = Math.floor(left + index * (width + innerGap));
            const end = Math.floor(left + index * (width + innerGap) + width);
            return { node, tiled: true, bounds: { x: start, y: top, width: end - start, height: bottom - top } };
        });
    }
}

const gnoblin = Gnoblin.get();
const clients = gnoblin.clients;
const context = gnoblin.input?.primary;
if (clients === undefined || context === undefined) {
    throw new Error("the core has no clients or no input");
}

const main = new Workspace("main", { name: "Main", layout: new Columns(), gaps: { inner: 10, outer: 20 } });
Workspaces.get().add(main);

clients.on("added", (client) => {
    console.log(`client ${client.id} opened on ${main.id}`);
});

context.bind("Super+Tab", () => {
    const focused = clients.toList().find((client) => client.focused);
    const first = main.clients?.members[0];
    if (focused === undefined || first === undefined) {
        return false;
    }
    main.clients?.swap(focused, first);
});
console.log("typescript std shell is running");
