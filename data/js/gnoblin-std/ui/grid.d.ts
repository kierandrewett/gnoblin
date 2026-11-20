import { Box } from "gnoblin-std/ui/box";
/**
 * Places children in a near-square grid of equal cells, with `padding` around the grid and `gap` between cells.
 * Cell sizes are rounded down to whole pixels; spare pixels remain along the right and bottom edges.
 */
export declare class Grid extends Box {
    /** Places children in equal near-square cells, rounding cell sizes down to logical pixels. */
    arrange(): void;
}
