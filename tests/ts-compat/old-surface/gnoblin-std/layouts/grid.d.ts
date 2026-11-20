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
    triggers?: {
        swap?: string | null;
    };
}
/** A near-square grid of the workspace's nodes. */
export declare class GridLayout extends NativeLayout {
    /** Fixed column count, or `"auto"` for a near-square grid. Default `"auto"`. */
    columns: "auto" | number;
    /** Order in which nodes fill the grid. Default `row`. */
    fill: GridFill;
    /** Gap between cells, or `undefined` to use the workspace gap. */
    gap: number | undefined;
    /** Resolved `swap` trigger; `null` means the binding is disabled. */
    readonly triggers: Readonly<Record<"swap", string | null>>;
    constructor(options?: GridOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
}
