import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
/** A `Box` as large as the frame, less the `bleed` of the layer it is in. `fill` and `radius` are the box's. */
export declare class FrameBackdrop extends Box {
    constructor(options?: BoxOptions);
    /** Fits the box to its frame layer, excluding any bleed. */
    refresh(): void;
}
