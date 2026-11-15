// `LayerOutlet`: the `underlay` behind a window's content and the `overlay` in front of it. A layer covers the whole
// frame and `bleed` pixels more on every side, takes no room and takes no pointer, and is not part of the frame's shape
// (the rounded corners, the border and a frame shadow do not grow to meet it). design/window-frame.md, decisions 6 and 7.

import type { OutletOwner } from "gnoblin/frame/outlet";
import { Outlet } from "gnoblin/frame/outlet";
import type { Node } from "gnoblin/scene/node";
import type { Canvas } from "gnoblin/ui";
import type { Rect } from "gnoblin/types";

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
export class LayerOutlet extends Outlet {
    readonly kind: LayerKind;
    /** How many pixels past the frame the layer reaches on every side. */
    bleed = 0;

    constructor(owner: OutletOwner, kind: LayerKind) {
        super(owner, kind === "under" ? "underlay" : "overlay");
        this.kind = kind;
        this.placement = kind;
        this.input = false;
    }

    override add(child: Canvas | Node): this {
        super.add(child);
        child.input = false;
        return this;
    }

    protected override place(): Rect {
        const { width, height, inset } = this.owner.geometry();
        return {
            x: -inset.left - this.bleed,
            y: -inset.top - this.bleed,
            width: inset.left + width + inset.right + 2 * this.bleed,
            height: inset.top + height + inset.bottom + 2 * this.bleed,
        };
    }
}
