import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";
/** The edge the master column sits on. */
export type Orientation = "left" | "right" | "top" | "bottom";
/** Options of `MasterStackLayout`. */
export interface MasterStackOptions {
    /** Share of the free width the master column takes, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio?: number;
    /** How many nodes share the master column. Default 1. */
    masterCount?: number;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** Which edge the master sits on. Default `left`. */
    orientation?: Orientation;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: {
        swap?: string | null;
        resize?: string | null;
    };
}
/** A tiling layout with one master column on one edge. The other nodes stack in the remaining column. */
export declare class MasterStackLayout extends NativeLayout {
    /** Share of the available master axis, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio: number;
    /** Number of nodes placed in the master region. Default 1. */
    masterCount: number;
    /** Gap between tiles, or `undefined` to use the workspace gap. */
    gap: number | undefined;
    /** Edge used for the master region. Default `left`. */
    orientation: Orientation;
    /** Resolved `swap` and `resize` triggers; `null` means that binding is disabled. */
    readonly triggers: Readonly<Record<"swap" | "resize", string | null>>;
    constructor(options?: MasterStackOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
}
