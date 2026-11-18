// StripLayout places one node per column in a horizontal strip, with focus kept in view. Carousel3dLayout arranges
// nodes on a perspective ring with the focused node at the front. Neither layout tiles its nodes. The columns, the view and
// the ring are the library's (`libgnoblin-std`, kinds `strip` and `carousel`); these classes hold the options.

import { BuiltinKind } from "gnoblin-std/generated/constants";
import type { LayoutNode } from "gnoblin-std/layout";
import { argOf, NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";

/** What the focus helpers need of a node: whether it has focus and how to give it focus. */
export interface FocusableNode {
    /** Whether this node currently has focus. */
    readonly focused: boolean;
    /** Gives focus to this node. */
    focus(): void;
}

/** What they need of a workspace: the nodes it holds. */
export interface NodeSource {
    /** Nodes available for focus navigation. */
    readonly clients: Iterable<LayoutNode>;
}

/** Options of `StripLayout`. */
export interface StripOptions {
    /** A column's width as a fraction of the strip's, held to 0.1 to 1. Default 0.5. */
    column?: number;
}

const STRIP_OPTIONS: OptionSpecs = {
    column: { native: "column" },
};

/**
 * Columns sit edge to edge with no gap or margin. The view follows focus unless `scrollTo` pinned it; a pin ends
 * when the focused node changes.
 *
 * The layout keeps its own column order, because a new node opens right after the focused column rather than at the
 * end. One instance must serve one workspace.
 */
export class StripLayout extends NativeLayout {
    /** Column width as a fraction of the strip width, clamped to 0.1..1. Default 0.5. */
    column: number;
    /** How many times `scrollTo` pinned the view: the pin is not a field the workspace can compare, so this counter is. */
    revision = 0;

    constructor(options: StripOptions = {}) {
        super();
        this.column = options.column ?? 0.5;
    }

    protected get kind(): number {
        return BuiltinKind.STRIP;
    }

    protected get options(): OptionSpecs {
        return STRIP_OPTIONS;
    }

    protected override changed(): void {
        this.revision += 1;
    }

    /** The view offset in pixels `scrollTo` pinned, or `null` while the view follows the focused column. */
    get pin(): number | null {
        const answer = this.act("pin", [], { clients: [] }, false);
        return typeof answer === "number" ? answer : null;
    }

    /** Scrolls the view to `offset` pixels and keeps it there until the focused node changes. */
    scrollTo(offset: number): void {
        this.act("scroll_to", [argOf("offset", offset)], { clients: [] }, false);
        this.changed();
    }

    /** Moves focus one column left; at the first column nothing happens. */
    focusLeft(workspace: NodeSource): void {
        this.act("focus_left", [], workspace);
    }

    /** Moves focus one column right; at the last column nothing happens. */
    focusRight(workspace: NodeSource): void {
        this.act("focus_right", [], workspace);
    }
}

/** Options of `Carousel3dLayout`. */
export interface RingOptions {
    /** Vertical field of view in degrees. Default 40. */
    fov?: number;
    /** A card's width as a fraction of the area's. Default 0.36. */
    cardWidth?: number;
    /** A card's height as a fraction of the area's. Default 0.6. */
    cardHeight?: number;
}

/** Options of `Carousel3dLayout`. It takes the same fields as `RingOptions`. */
export type Carousel3dOptions = RingOptions;

const CAROUSEL_OPTIONS: OptionSpecs = {
    fov: { native: "fov" },
    cardWidth: { native: "card_width" },
    cardHeight: { native: "card_height" },
};

/**
 * Cards on a ring, the focused node at the front; with no focus among the nodes the first is. Each placement is the
 * ring's projected rectangle together with the ring's depth (`z`) and its angle as a rotation about the vertical
 * axis, so a renderer that draws depth can use them. The depth fade of the ring is not applied: a node's opacity
 * belongs to the rules.
 */
export class Carousel3dLayout extends NativeLayout {
    /** Vertical field of view, in degrees. Default 40. */
    fov: number;
    /** Card width as a fraction of the area's width. Default 0.36. */
    cardWidth: number;
    /** Card height as a fraction of the area's height. Default 0.6. */
    cardHeight: number;

    constructor(options: Carousel3dOptions = {}) {
        super();
        this.fov = options.fov ?? 40;
        this.cardWidth = options.cardWidth ?? 0.36;
        this.cardHeight = options.cardHeight ?? 0.6;
    }

    protected get kind(): number {
        return BuiltinKind.CAROUSEL;
    }

    protected get options(): OptionSpecs {
        return CAROUSEL_OPTIONS;
    }

    /** Moves focus `steps` cards round the ring, wrapping at both ends. */
    rotate(workspace: NodeSource, steps: number): void {
        this.act("rotate", [argOf("steps", steps)], workspace);
    }
}
