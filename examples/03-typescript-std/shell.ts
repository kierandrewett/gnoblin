// Example 3 in TypeScript: a shell on the generated `gnoblin-std/native` binding of `libgnoblin-std`.
//
//     gnoblin ./shell.ts
//
// The same shell as examples 3 (Python) and 6 (Rust), in TypeScript:
//
// - `Columns` is a `Layout`: given the clients of a workspace and its area, it answers with where each goes, one column each, the
//   newest on the right. The library calls it whenever the workspace changes and writes what it answers;
// - `createWorkspace` makes the one workspace, with a gap of 10 between tiles and 20 to the screen's edge;
// - a listener on the collection of clients says where each window opens;
// - `Super+Tab` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.
import { KeyBinding, gnoblin } from "gnoblin-core";
import { EventKind, clients, createWorkspace, insetArea, tile } from "gnoblin-std/native";
import type { Layout, LayoutContext } from "gnoblin-std/native";

/** Equal-width columns across the workspace, separated by the workspace's inner gap. */
class Columns implements Layout {
    compute(cx: LayoutContext) {
        const count = cx.children.length;
        if (count === 0) {
            return [];
        }
        const area = insetArea(cx);
        const width = Math.max((area.width - cx.gapInner * (count - 1)) / count, 0);
        return cx.children.map((client, index) =>
            tile(client, area.x + index * (width + cx.gapInner), area.y, width, area.height),
        );
    }
}

const main = createWorkspace("main", { name: "Main", layout: new Columns(), gaps: [10, 20] });

clients().listen(EventKind.ADDED, 0, (event) => {
    console.log(`client ${event.member} opened on ${main.info().id}`);
});

class SwapWithFirst extends KeyBinding {
    override onPress(): void | false {
        const focused = gnoblin.input.primary.focusedClient;
        const first = main.clients().members()[0];
        if (focused === undefined || first === undefined) {
            return false;
        }
        main.clients().swap(Number(focused.id), first);
    }
}

gnoblin.input.primary.bind("Super+Tab", new SwapWithFirst({ id: "swap-with-first", trigger: "Super+Tab" }));
console.log("typescript std shell is running");
