// Example 5: a TypeScript shell with types and imports and exports across three files.
//
//     gnoblin ./shell.ts
//
// The types are erased when the file loads; nothing compiles ahead of time. Run `gnoblinctl types init` once for an editor that
// knows `gnoblin-core`. An error in any of the three files is reported at the line of the `.ts` file that wrote it.
import { ClientCollectionListener, gnoblin } from "gnoblin-core";
import type { Client } from "gnoblin-core";

import { explode, split } from "./geometry.ts";
import { Gap } from "./types.ts";
import type { Arrangement } from "./types.ts";

let arrangement: Arrangement = "columns";

/** Puts every window in its piece of the first output. */
function tile(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows: Client[] = gnoblin.clients.toList();
    const pieces = split(
        { x: output.x, y: output.y, width: output.width, height: output.height },
        windows.length,
        arrangement,
        Gap.Large,
    );
    windows.forEach((client, at) => {
        client.bounds = pieces[at];
    });
}

class Tiler extends ClientCollectionListener {
    onAdded(_client: Client): void {
        tile();
    }
    onRemoved(_client: Client): void {
        tile();
    }
}
gnoblin.clients.addListener(new Tiler());

// Super+t switches between columns and rows.
gnoblin.input.primary.bind("Super+t", () => {
    arrangement = arrangement === "columns" ? "rows" : "columns";
    tile();
});

// Super+x raises an error in `geometry.ts`, to show where it is reported.
gnoblin.input.primary.bind("Super+x", () => explode("example 5 stops here"));
