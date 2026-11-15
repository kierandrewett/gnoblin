// `EdgeOutlet` is a strip along one side of a window frame. Its thickness is free space around the content, so nodes in
// it do not overlap the client. The strip has start, center and end zones.
// the strip do not overlap the client. The strip has start, center and end zones.

import type { FrameGeometry, OutletOwner } from "gnoblin/frame/outlet";
import { Outlet } from "gnoblin/frame/outlet";
import type { Node } from "gnoblin/scene/node";
import type { Canvas } from "gnoblin/ui/canvas";
import type { Rect } from "gnoblin/types";

/** The four edges of a frame. */
export type Side = "top" | "right" | "bottom" | "left";

/** The three zones of an edge, in order along it. */
export interface EdgeZones {
    /** The first end of the edge: the left of `top`, the top of `left`. Takes the size of its nodes. */
    readonly start: Outlet;
    /** What is between: takes the room that is left. */
    readonly center: Outlet;
    /** The last end of the edge. Takes the size of its nodes. */
    readonly end: Outlet;
}

/** The rectangle of an edge strip in the window's coordinates, from the content's size and the inset. */
function edgeRect(side: Side, { width, height, inset }: FrameGeometry): Rect {
    switch (side) {
        case "top":
            return { x: -inset.left, y: -inset.top, width: inset.left + width + inset.right, height: inset.top };
        case "bottom":
            return { x: -inset.left, y: height, width: inset.left + width + inset.right, height: inset.bottom };
        case "left":
            return { x: -inset.left, y: 0, width: inset.left, height };
        case "right":
            return { x: width, y: 0, width: inset.right, height };
    }
}

/**
 * An edge of the frame. `size` is how much room it takes: the content moves in by that much. Left `null` the edge is
 * as thick as the thickest node was when it was added (its height for `top` and `bottom`, its width for `left` and
 * `right`), so give a node that is measured later (a `Text`) an assigned size or set `size` yourself. The strip
 * spans the whole width of the frame for `top` and `bottom`, and the content's height for `left` and `right`.
 *
 * `start`, `center` and `end` are the three zones of the edge, a titlebar's left buttons, its title and its right
 * buttons: reading one makes all three. Nodes added to the edge itself sit beside them.
 */
export class EdgeOutlet extends Outlet {
    /** The side of the window frame this strip follows. */
    readonly side: Side;
    /** The thickness in pixels, or `null` to measure the nodes added. */
    size: number | null = null;
    #measured = 0;
    #zones: EdgeZones | null = null;

    constructor(owner: OutletOwner, side: Side) {
        super(owner, side, { direction: side === "top" || side === "bottom" ? "row" : "column" });
        this.side = side;
        this.onAdded = (child) => this.noteThickness(child);
    }

    /** The room the edge takes while the window is not fullscreen: 0 when the edge is hidden. */
    get reserved(): number {
        return this.enabled ? (this.size ?? this.#measured) : 0;
    }

    /** The room the edge takes now: nothing while the window is fullscreen. */
    get thickness(): number {
        return this.shown ? this.reserved : 0;
    }

    /** The first zone of the edge. */
    get start(): Outlet {
        return this.zones().start;
    }

    /** The middle zone of the edge, which takes the room the other two leave. */
    get center(): Outlet {
        return this.zones().center;
    }

    /** The last zone of the edge. */
    get end(): Outlet {
        return this.zones().end;
    }

    /** Makes the three zones the first time one is read. */
    protected zones(): EdgeZones {
        if (this.#zones === null) {
            const direction = this.direction;
            const make = (name: string, grow: number): Outlet => {
                const zone = new Outlet(this.owner, `${this.side}.${name}`, { direction, grow });
                zone.onAdded = (child) => this.noteThickness(child);
                super.add(zone);
                return zone;
            };
            this.#zones = { start: make("start", 0), center: make("center", 1), end: make("end", 0) };
        }
        return this.#zones;
    }

    /** Destroys what was added, the zones with it, and forgets how thick the nodes were. */
    override clear(): void {
        super.clear();
        this.#zones = null;
        this.#measured = 0;
    }

    /** Learns how thick a node added to the edge or to one of its zones is, if the edge has no size of its own. */
    protected noteThickness(child: Canvas | Node): void {
        const along = this.side === "top" || this.side === "bottom";
        const own = along ? ("height" in child ? child.height : undefined) : "width" in child ? child.width : undefined;
        if (typeof own === "number") {
            this.#measured = Math.max(this.#measured, own);
        }
    }

    protected override place(): Rect {
        return edgeRect(this.side, this.owner.geometry());
    }
}
