// MasterStackLayout places `masterCount` nodes in a master column and the rest in a stack. The master takes `ratio`
// of the available width (or height for top and bottom); `gap` separates tiles. The algorithm is the library's
// (`libgnoblin-std`, kind `master_stack`); this class holds the options and the bindings.
// Default `swap` and `resize` bindings trade tile positions and drag the divider between the master and stack.
// `triggers` replaces a binding trigger, and `null` leaves that binding out.

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { addTilingBindings } from "gnoblin-std/bindings/tiling";
import { BuiltinKind } from "gnoblin-std/generated/constants";
import { MasterStackOrientation } from "gnoblin-std/generated/layouts";
import { checkGap } from "gnoblin-std/layout";
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
    triggers?: { swap?: string | null; resize?: string | null };
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

const OPTIONS: OptionSpecs = {
    ratio: { native: "ratio" },
    masterCount: { native: "master_count" },
    gap: { native: "gap" },
    orientation: { native: "orientation", members: MasterStackOrientation },
};

/** A tiling layout with one master column on one edge. The other nodes stack in the remaining column. */
export class MasterStackLayout extends NativeLayout {
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

    constructor(options: MasterStackOptions = {}) {
        super();
        this.ratio = options.ratio ?? 0.5;
        this.masterCount = options.masterCount ?? 1;
        this.gap = checkGap(options.gap, "MasterStackLayout");
        this.orientation = options.orientation ?? "left";
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        addTilingBindings(this.bindings, this.triggers);
    }

    protected get kind(): number {
        return BuiltinKind.MASTER_STACK;
    }

    protected get options(): OptionSpecs {
        return OPTIONS;
    }
}
