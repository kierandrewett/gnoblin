// Where an output goes against its neighbour, and the name a message gives an output.

import type { OutputAlign, OutputView } from "gnoblin-std/display/types";

/** The name a message gives an output. */
export function nameOf(output: OutputView): string {
    return output.connector === "" ? output.name : output.connector;
}

/** The top-left corner that puts an output of `size` against `target` on `side`, aligned as asked. */
export function placed(
    side: "rightOf" | "leftOf" | "above" | "below",
    align: OutputAlign | undefined,
    target: OutputView,
    size: { width: number; height: number },
): { x: number; y: number } {
    const horizontal = side === "rightOf" || side === "leftOf";
    // Default alignment starts at the top or left end of the shared edge.
    const along = (own: number, theirs: number, at: OutputAlign | undefined): number => {
        if (at === "center") return Math.round((theirs - own) / 2);
        if (at === "bottom" || at === "right") return theirs - own;
        return 0;
    };
    if (horizontal) {
        return {
            x: side === "rightOf" ? target.x + target.width : target.x - size.width,
            y: target.y + along(size.height, target.height, align),
        };
    }
    return {
        x: target.x + along(size.width, target.width, align),
        y: side === "below" ? target.y + target.height : target.y - size.height,
    };
}
