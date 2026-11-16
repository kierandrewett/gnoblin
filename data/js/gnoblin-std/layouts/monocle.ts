// MonocleLayout shows the focused node at full size and hides the others without moving them. If focus is elsewhere,
// it shows the first child. Hidden nodes remain in the tree and are tiled again when shown. See `gnoblin-std/layout` for
// the `Layout` base class.

import { Layout, tile, tilingGeometry } from "gnoblin-std/layout";
import type { LayoutContext, Placement } from "gnoblin-std/layout";

/**
 * Shows one node full size and hides the rest. A hidden node keeps its place in the tree, so it is tiled again when
 * shown.
 */
export class MonocleLayout extends Layout {
    /** Returns full-area placements, with only the focused child visible. */
    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        if (nodes.length === 0) {
            return [];
        }
        const { x, y, width, height } = tilingGeometry(undefined, context).area;
        const focused = context.focused;
        const shown = focused !== null && focused !== undefined && nodes.includes(focused) ? focused : nodes[0];
        return nodes.map((node) => ({ ...tile(node, x, y, width, height), visible: node === shown }));
    }
}
