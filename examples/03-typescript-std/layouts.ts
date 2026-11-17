// The built-in layouts of `gnoblin-std/native` from TypeScript: one workspace and a chord for each tiling layout the library runs.
//
//     gnoblin ./layouts.ts
//
// `Super+1` is a default master and stack, `Super+2` a wider master area of two windows on the right with its own gap, `Super+3` an
// automatic grid, `Super+4` a grid of three columns filled by column, `Super+5` a monocle and `Super+6` a master and stack with the
// master on top. `Super+Button1` swaps a tiled window with the tile it is dropped on.
//
// Every class here is generated from the library's table of layout kinds (`gnoblin-std-abi`'s `LAYOUT_KINDS`): the options, their
// defaults and their enumerations are the library's, and so is the algorithm. The shell only names a kind and its options.
import { KeyBinding, gnoblin } from "gnoblin-core";
import {
    GridFill,
    GridLayout,
    MasterStackLayout,
    MasterStackOrientation,
    MonocleLayout,
    SwapBinding,
    createWorkspace,
} from "gnoblin-std/native";
import type { BuiltinLayout } from "gnoblin-std/native";

const main = createWorkspace("main", { gaps: [6, 12] });
new MasterStackLayout().attach(main);

const layouts: Record<string, () => BuiltinLayout> = {
    "Super+1": () => new MasterStackLayout(),
    "Super+2": () =>
        new MasterStackLayout({ ratio: 0.6, masterCount: 2, gap: 4, orientation: MasterStackOrientation.RIGHT }),
    "Super+3": () => new GridLayout(),
    "Super+4": () => new GridLayout({ columns: 3, fill: GridFill.COLUMN }),
    "Super+5": () => new MonocleLayout(),
    "Super+6": () => new MasterStackLayout({ orientation: MasterStackOrientation.TOP }),
};

class Switch extends KeyBinding {
    constructor(
        trigger: string,
        private readonly make: () => BuiltinLayout,
    ) {
        super({ id: `layout-${trigger}`, trigger });
    }

    override onPress(): void | false {
        this.make().attach(main);
    }
}

for (const [chord, make] of Object.entries(layouts)) {
    gnoblin.input.primary.bind(chord, new Switch(chord, make));
}

new SwapBinding().start();
console.log("layouts shell is running");
