// `FrameBackdrop`: a fill behind the whole frame. In the underlay it shows through a client that draws translucent
// pixels, and it is what a frame with no titlebar leaves to fill the gap.

import { LayerOutlet } from "gnoblin/frame/layer_outlet";
import { Box } from "gnoblin/ui";
import type { BoxOptions } from "gnoblin/ui";

/** A `Box` as large as the frame, less the `bleed` of the layer it is in. `fill` and `radius` are the box's. */
export class FrameBackdrop extends Box {
    constructor(options: BoxOptions = {}) {
        super({ fill: "#1e1e2e", ...options });
    }

    override refresh(): void {
        const { parent } = this;
        if (parent !== null) {
            const bleed = parent instanceof LayerOutlet ? parent.bleed : 0;
            this.x = bleed;
            this.y = bleed;
            this.width = parent.width - 2 * bleed;
            this.height = parent.height - 2 * bleed;
        }
    }
}
