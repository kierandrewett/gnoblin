// The geometry of example 5: dividing an area among windows.
import type { Area, Arrangement } from "./types.ts";
import { Gap } from "./types.ts";

/** Divides `area` into `count` pieces side by side (`columns`) or one above another (`rows`), `gap` apart. */
export function split(area: Area, count: number, arrangement: Arrangement = "columns", gap: Gap = Gap.Small): Area[] {
    if (count <= 0) {
        return [];
    }
    const between = gap * (count - 1);
    const pieces: Area[] = [];
    for (let at = 0; at < count; at++) {
        if (arrangement === "columns") {
            const width = (area.width - between) / count;
            pieces.push({ x: area.x + at * (width + gap), y: area.y, width, height: area.height });
        } else {
            const height = (area.height - between) / count;
            pieces.push({ x: area.x, y: area.y + at * (height + gap), width: area.width, height });
        }
    }
    return pieces;
}

/** Stops the shell's key handler with `reason`: the error names this file and this line, not the stripped JavaScript. */
export function explode(reason: string): never {
    throw new Error(reason);
}
