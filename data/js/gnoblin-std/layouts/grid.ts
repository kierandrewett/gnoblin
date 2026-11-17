// GridLayout places nodes in child order in a near-square grid. `fill` chooses row-wise or column-wise order, and
// `columns` can fix the column count. The `swap` binding trades two cells; there is no divider to resize.
// `triggers` replaces the binding trigger, and `null` leaves the binding out. The algorithm is the library's
// (`libgnoblin-std`, kind `grid`).

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { addTilingBindings } from "gnoblin-std/bindings/tiling";
import { BuiltinKind } from "gnoblin-std/generated/constants";
import { GridFill as GridFillMembers } from "gnoblin-std/generated/layouts";
import { checkGap } from "gnoblin-std/layout";
import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";

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

const OPTIONS: OptionSpecs = {
    // The library's 0 is the near-square grid.
    columns: {
        native: "columns",
        toNative: (value) => {
            if (value === "auto") {
                return 0;
            }
            if (typeof value !== "number" || !Number.isInteger(value) || value < 1) {
                throw new RangeError(`GridLayout columns must be "auto" or a whole number of at least 1`);
            }
            return value;
        },
        fromNative: (value) => (value === 0 ? "auto" : value),
    },
    fill: { native: "fill", members: GridFillMembers },
    gap: { native: "gap" },
};

/** A near-square grid of the workspace's nodes. */
export class GridLayout extends NativeLayout {
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
        addTilingBindings(this.bindings, this.triggers);
    }

    protected get kind(): number {
        return BuiltinKind.GRID;
    }

    protected get options(): OptionSpecs {
        return OPTIONS;
    }
}
