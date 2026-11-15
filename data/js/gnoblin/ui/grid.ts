// `Grid`, a box whose children sit in a near-square grid of equal cells.

import { Box } from "gnoblin/ui/box";

/**
 * Children in a near-square grid of equal cells, `padding` around and `gap` between. Cells are floored to whole pixels,
 * so the spare pixels collect along the right and bottom edges.
 */
export class Grid extends Box {
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
