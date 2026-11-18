// StripLayout places one node per column in a horizontal strip, with focus kept in view. Carousel3dLayout arranges
// nodes on a perspective ring with the focused node at the front. Neither layout tiles its nodes. The columns, the view and
// the ring are the library's (`libgnoblin-std`, kinds `strip` and `carousel`); these classes hold the options.

import { Builtin } from "gnoblin-std/generated/classes";
import { BuiltinKind } from "gnoblin-std/generated/constants";
import type { Rect } from "gnoblin-core/types";
import type { Placement as LibraryPlacement } from "gnoblin-std/generated/records";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";
import { argOf, NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";

/** The built-in layout `kind` made with `options`, asked once for the placements of `count` cards, and freed. */
function computeOnce(
    kind: number,
    options: ReturnType<typeof argOf>[],
    count: number,
    focus: number,
    area: Rect,
): ReturnType<Builtin["compute"]> {
    const made = Builtin.new(kind, options);
    if (made === null) {
        throw new Error("the library made no layout");
    }
    try {
        const children = Array.from({ length: count }, (_, i) => i + 1);
        return made.compute({
            flags: 0,
            children,
            area,
            gapInner: 0,
            gapOuter: 0,
            focused: focus,
            pointerX: 0,
            pointerY: 0,
            focusedWidth: 0,
            focusedHeight: 0,
        });
    } finally {
        made.free();
    }
}

/**
 * A column's width as a fraction of the strip's, held to the library's 0.1 to 1. Anything but a number is an error. The bounds are the
 * library's: a one-column strip over a width of 1 is as wide as the column the library would give.
 */
export function clampColumn(value: unknown): number {
    if (typeof value !== "number" || Number.isNaN(value)) {
        throw new TypeError(`column must be a number, got ${String(value)}`);
    }
    // The library takes a finite number below the range of a script's node numbers, and holds it to its own: an infinity is as far outside
    // it as 2 or -1 are.
    const finite = Math.min(Math.max(value, -1), 2);
    const [placed] = computeOnce(BuiltinKind.STRIP, [argOf("column", finite)], 1, 1, {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    return placed.bounds.width;
}

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
        clampColumn(this.column);
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

    /** Focus moves to a neighbour of a focused column that went, so the strip keeps a focus. */
    onClientRemoved(client: LayoutNode, context: LayoutContext): void {
        this.handOnFocus(client, context);
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

/** One card of the ring: the projected rectangle, the depth fade, the angle round the ring and the depth. */
export interface RingSlot extends Rect {
    /** Fade from 0.4 at the back to 1 at the front. */
    opacity: number;
    /** Degrees: 0 at the front, 180 at the back, positive to the right of the front card. */
    rotation: number;
    /** Negative in front of the camera, more so for the back of the ring. */
    z: number;
}

/**
 * The pure ring layout of `count` cards with the 1-based `focus` at the front, inside `area`. One slot per card. The rectangles, the
 * depth and the angle are the library's (kind `carousel`); the fade is a function of the angle, which is all the library does not
 * hold: a node's opacity belongs to the rules, so a layout never applies it.
 */
export function ring(count: number, focus: number, area: Rect, options: RingOptions = {}): RingSlot[] {
    if (count === 0) {
        return [];
    }
    if (!Number.isInteger(focus) || focus < 1 || focus > count) {
        throw new RangeError(`focus must be an index in 1..${count}, got ${String(focus)}`);
    }
    const list = [
        argOf("fov", options.fov ?? 40),
        argOf("card_width", options.cardWidth ?? 0.36),
        argOf("card_height", options.cardHeight ?? 0.6),
    ];
    return computeOnce(BuiltinKind.CAROUSEL, list, count, focus, area).map((placement) => {
        const angle = placement.rotationY;
        return {
            ...placement.bounds,
            opacity: 0.4 + (0.6 * (1 + Math.cos(angle))) / 2,
            rotation: (angle * 180) / Math.PI,
            z: placement.z,
        };
    });
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

    /** A card always has its turn about the vertical axis, a front card's none. */
    protected override placement(raw: LibraryPlacement): Placement {
        const placement = super.placement(raw);
        placement.rotation ??= [0, 0, 0];
        return placement;
    }

    /** Moves focus `steps` cards round the ring, wrapping at both ends. */
    rotate(workspace: NodeSource, steps: number): void {
        this.act("rotate", [argOf("steps", steps)], workspace);
    }
}
