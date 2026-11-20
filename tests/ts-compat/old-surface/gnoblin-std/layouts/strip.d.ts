import type { Rect } from "gnoblin-core/types";
import type { Placement as LibraryPlacement } from "gnoblin-std/generated/records";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";
import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";
/**
 * A column's width as a fraction of the strip's, held to the library's 0.1 to 1. Anything but a number is an error. The bounds are the
 * library's: a one-column strip over a width of 1 is as wide as the column the library would give.
 */
export declare function clampColumn(value: unknown): number;
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
/**
 * Columns sit edge to edge with no gap or margin. The view follows focus unless `scrollTo` pinned it; a pin ends
 * when the focused node changes.
 *
 * The layout keeps its own column order, because a new node opens right after the focused column rather than at the
 * end. One instance must serve one workspace.
 */
export declare class StripLayout extends NativeLayout {
    #private;
    /** Column width as a fraction of the strip width, clamped to 0.1..1. Default 0.5. */
    column: number;
    /** How many times `scrollTo` pinned the view: the pin is not a field the workspace can compare, so this counter is. */
    revision: number;
    constructor(options?: StripOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
    protected changed(): void;
    /** Focus moves to a neighbour of a focused column that went, so the strip keeps a focus. */
    onClientRemoved(client: LayoutNode, context: LayoutContext): void;
    /** The view offset in pixels `scrollTo` pinned, or `null` while the view follows the focused column. Setting it pins the view, or with `null` lets it follow again. */
    get pin(): number | null;
    set pin(offset: number | null);
    /** Scrolls the view to `offset` pixels and keeps it there until the focused node changes. */
    scrollTo(offset: number): void;
    /** Moves focus one column left; at the first column nothing happens. */
    focusLeft(workspace: NodeSource): void;
    /** Moves focus one column right; at the last column nothing happens. */
    focusRight(workspace: NodeSource): void;
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
export declare function ring(count: number, focus: number, area: Rect, options?: RingOptions): RingSlot[];
/** Options of `Carousel3dLayout`. It takes the same fields as `RingOptions`. */
export type Carousel3dOptions = RingOptions;
/**
 * Cards on a ring, the focused node at the front; with no focus among the nodes the first is. Each placement is the
 * ring's projected rectangle together with the ring's depth (`z`) and its angle as a rotation about the vertical
 * axis, so a renderer that draws depth can use them. The depth fade of the ring is not applied: a node's opacity
 * belongs to the rules.
 */
export declare class Carousel3dLayout extends NativeLayout {
    /** Vertical field of view, in degrees. Default 40. */
    fov: number;
    /** Card width as a fraction of the area's width. Default 0.36. */
    cardWidth: number;
    /** Card height as a fraction of the area's height. Default 0.6. */
    cardHeight: number;
    constructor(options?: Carousel3dOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
    /** A card always has its turn about the vertical axis, a front card's none. */
    protected placement(raw: LibraryPlacement): Placement;
    /** Moves focus `steps` cards round the ring, wrapping at both ends. */
    rotate(workspace: NodeSource, steps: number): void;
}
