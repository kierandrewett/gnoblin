// GridLayout places nodes in child order in a near-square grid. `fill` chooses row-wise or column-wise order, and
// `columns` can fix the column count. The `swap` binding trades two cells; there is no divider to resize.
// `triggers` replaces the binding trigger, and `null` leaves the binding out.

import { resolveTriggers } from "gnoblin/bindings/binding";
import { ClientSwapBinding } from "gnoblin/bindings/tiling";
import { checkGap, Layout, tile, tilingGeometry } from "gnoblin/layout";
import type { LayoutContext, Placement } from "gnoblin/layout";

/** How nodes fill the grid: left to right then down (`"row"`, the default), or top to bottom then right. */
export type GridFill = "row" | "column";

/** The options of `GridLayout`. Every field is optional. */
export interface GridOptions {
    /** `"auto"` picks a near-square grid; a number fixes the column count. Default `"auto"`. */
    columns?: "auto" | number;
    /** Fills rows left to right or columns top to bottom. Default `row`. */
    fill?: GridFill;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: { swap?: string | null };
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag" } as const;

/** A near-square grid of the workspace's nodes. */
export class GridLayout extends Layout {
    /** Fixed column count, or `"auto"` for a near-square grid. Default `"auto"`. */
    columns: "auto" | number;
    /** Order in which nodes fill the grid. Default `row`. */
    fill: GridFill;
    /** Gap between cells, or `undefined` to use the workspace gap. */
    gap: number | undefined;
    /** Resolved `swap` trigger; `null` means the binding is disabled. */
    readonly triggers: Readonly<Record<"swap", string | null>>;

    constructor(options: GridOptions = {}) {
        super();
        this.columns = options.columns ?? "auto";
        this.fill = options.fill ?? "row";
        this.gap = checkGap(options.gap, "GridLayout");
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        if (this.triggers.swap !== null) {
            this.bindings.add(new ClientSwapBinding({ id: "swap", trigger: this.triggers.swap }));
        }
    }

    /** Returns one tiled placement per child in the selected grid order. */
    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        const n = nodes.length;
        if (n === 0) {
            return [];
        }
        if (this.fill !== "row" && this.fill !== "column") {
            throw new TypeError("GridLayout fill must be row or column");
        }
        const { cols, rows } = this.shape(n);
        const { area, gap } = tilingGeometry(this.gap, context);
        const { x, y, width, height } = area;
        const cellWidth = Math.max(0, (width - (cols - 1) * gap) / cols);
        const cellHeight = Math.max(0, (height - (rows - 1) * gap) / rows);
        return nodes.map((node, i) => {
            const col = this.fill === "column" ? Math.floor(i / rows) : i % cols;
            const row = this.fill === "column" ? i % rows : Math.floor(i / cols);
            return tile(node, x + col * (cellWidth + gap), y + row * (cellHeight + gap), cellWidth, cellHeight);
        });
    }

    /** Columns and rows for `n` nodes. */
    private shape(n: number): { cols: number; rows: number } {
        if (this.columns !== "auto") {
            if (!Number.isInteger(this.columns) || this.columns < 1) {
                throw new RangeError(`GridLayout columns must be "auto" or a whole number of at least 1`);
            }
            return { cols: this.columns, rows: Math.ceil(n / this.columns) };
        }
        const side = Math.ceil(Math.sqrt(n));
        return this.fill === "column"
            ? { cols: Math.ceil(n / side), rows: side }
            : { cols: side, rows: Math.ceil(n / side) };
    }
}
