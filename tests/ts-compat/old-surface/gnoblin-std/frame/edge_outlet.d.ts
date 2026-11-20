import type { OutletOwner } from "gnoblin-std/frame/outlet";
import { Outlet } from "gnoblin-std/frame/outlet";
import type { Node } from "gnoblin-std/scene/node";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { Rect } from "gnoblin-core/types";
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
/**
 * An edge of the frame. `size` is how much room it takes: the content moves in by that much. Left `null` the edge is
 * as thick as the thickest node was when it was added (its height for `top` and `bottom`, its width for `left` and
 * `right`), so give a node that is measured later (a `Text`) an assigned size or set `size` yourself. The strip
 * spans the whole width of the frame for `top` and `bottom`, and the content's height for `left` and `right`.
 *
 * `start`, `center` and `end` are the three zones of the edge, a titlebar's left buttons, its title and its right
 * buttons: reading one makes all three. Nodes added to the edge itself sit beside them.
 */
export declare class EdgeOutlet extends Outlet {
    #private;
    /** The side of the window frame this strip follows. */
    readonly side: Side;
    /** The thickness in pixels, or `null` to measure the nodes added. */
    size: number | null;
    constructor(owner: OutletOwner, side: Side);
    /** The room the edge takes while the window is not fullscreen: 0 when the edge is hidden. */
    get reserved(): number;
    /** The room the edge takes now: nothing while the window is fullscreen. */
    get thickness(): number;
    /** The first zone of the edge. */
    get start(): Outlet;
    /** The middle zone of the edge, which takes the room the other two leave. */
    get center(): Outlet;
    /** The last zone of the edge. */
    get end(): Outlet;
    /** Makes the three zones the first time one is read. */
    protected zones(): EdgeZones;
    /** Destroys what was added, the zones with it, and forgets how thick the nodes were. */
    clear(): void;
    /** Learns how thick a node added to the edge or to one of its zones is, if the edge has no size of its own. */
    protected noteThickness(child: Canvas | Node): void;
    protected place(): Rect;
}
