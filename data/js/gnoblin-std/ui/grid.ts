// A box layout that places children in a near-square grid of equal cells.

import { Box } from "gnoblin-std/ui/box";

/**
 * Places children in a near-square grid of equal cells, with `padding` around the grid and `gap` between cells.
 * Cell sizes are rounded down to whole pixels; spare pixels remain along the right and bottom edges.
 */
export class Grid extends Box {
    /** Places children in equal near-square cells, rounding cell sizes down to logical pixels. */
    override arrange(): void {
        const kids = this.children;
        const pad = this.padding;
        const gap = this.gap;
        const cols = Math.max(1, Math.ceil(Math.sqrt(kids.length)));
        const rows = Math.max(1, Math.ceil(kids.length / cols));
        const w = Math.floor((this.width - 2 * pad - (cols - 1) * gap) / cols);
        const h = Math.floor((this.height - 2 * pad - (rows - 1) * gap) / rows);
        kids.forEach((kid, i) => {
            kid.x = pad + (i % cols) * (w + gap);
            kid.y = pad + Math.floor(i / cols) * (h + gap);
            kid.width = w;
            kid.height = h;
        });
    }
}
