// MonocleLayout: the shown node fills the area and every other node is hidden, not moved. The shown node
// is the focused one when it is a child, otherwise the first child. Geometry is data/lua/gnoblin/shell/tiling.lua
// `Monocle`.

import { Layout, tile, tilingGeometry } from "gnoblin/layout";
import type { LayoutContext, Placement } from "gnoblin/layout";

export class MonocleLayout extends Layout {
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
