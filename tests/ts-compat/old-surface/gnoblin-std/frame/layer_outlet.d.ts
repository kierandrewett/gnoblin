import type { OutletOwner } from "gnoblin-std/frame/outlet";
import { Outlet } from "gnoblin-std/frame/outlet";
import type { Node } from "gnoblin-std/scene/node";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { Rect } from "gnoblin-core/types";
/** Which side of the window's content a layer sits. */
export type LayerKind = "under" | "over";
/**
 * A layer of the frame. The origin of its coordinates is its own corner, `bleed` pixels outside the frame's corner,
 * so a node at `x: bleed` starts where the frame does. The layer lets the pointer through: a canvas added to it has its
 * `input` set to `false`, and a script that wants a node to take the pointer (a badge to click, a resize handle) sets
 * it back to `true` after `add`.
 *
 * A glow or a drop shadow that reaches outside the frame sets `bleed` to how far, and is painted into the room
 * that gives. It is painted whole, though a part of a layer outside the window's output is not seen.
 */
export declare class LayerOutlet extends Outlet {
    /** Whether the layer sits behind or in front of the window content. */
    readonly kind: LayerKind;
    /** How many pixels past the frame the layer reaches on every side. */
    bleed: number;
    constructor(owner: OutletOwner, kind: LayerKind);
    /** Adds a node and disables pointer input on it. */
    add(child: Canvas | Node): this;
    protected place(): Rect;
}
