// The built-in layouts of `gnoblin-std` from TypeScript: one workspace and a chord for each tiling layout the library runs.
//
//     gnoblin ./layouts.ts
//
// `Super+1` is a default master and stack, `Super+2` a wider master area of two windows on the right with its own gap, `Super+3` an
// automatic grid, `Super+4` a grid of three columns filled by column, `Super+5` a monocle and `Super+6` a master and stack with the
// master on top. `Super+Button1` swaps a tiled window with the tile it is dropped on.
//
// Every class here is generated from the library's table of layout kinds (`gnoblin-std-abi`'s `LAYOUT_KINDS`): the options, their
// defaults and their enumerations are the library's, and so is the algorithm. The shell only names a kind and its options.
import { Gnoblin } from "gnoblin-core";
import { GridLayout, MasterStackLayout, MonocleLayout, SwapBinding, Workspace, Workspaces } from "gnoblin-std";
import type { Layout } from "gnoblin-std";

const context = Gnoblin.get().input?.primary;
if (context === undefined) {
    throw new Error("the core has no input");
}

const main = new Workspace("main", { gaps: { inner: 6, outer: 12 }, layout: new MasterStackLayout() });
Workspaces.get().add(main);

const layouts: Record<string, () => Layout> = {
    "Super+1": () => new MasterStackLayout(),
    "Super+2": () => new MasterStackLayout({ ratio: 0.6, masterCount: 2, gap: 4, orientation: "right" }),
    "Super+3": () => new GridLayout(),
    "Super+4": () => new GridLayout({ columns: 3, fill: "column" }),
    "Super+5": () => new MonocleLayout(),
    "Super+6": () => new MasterStackLayout({ orientation: "top" }),
};

for (const [chord, make] of Object.entries(layouts)) {
    context.bind(chord, () => {
        main.layout = make();
    });
}

new SwapBinding().install();
console.log("layouts shell is running");
