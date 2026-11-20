import { BindingCollection } from "gnoblin-core/bindings/binding";
import type { RingResult, RingView, XRRingParams } from "gnoblin-std/extensions/xr/ring";
/** What `new SpatialLayout` takes; every distance is in metres. */
export interface SpatialLayoutOptions {
    /** The distance from the viewer to the centre of a panel, in metres. Default 1.6. */
    radius?: number;
    /** The height of a panel's centre above the floor, in metres. Default 1.4. */
    eye?: number;
    /** The angle between neighbouring panels, in radians. Default 3 degrees. */
    spacing?: number;
    /** The metres of panel for one pixel of window. Default 0.00075. */
    metresPerPixel?: number;
    /** The focused panel is this much larger. Default 1.15. */
    focusScale?: number;
    /** ... and this many metres closer. Default 0.1. */
    focusPull?: number;
    /** The most the panels reach to either side of the front, in radians. Default 150 degrees. */
    halfArc?: number;
    /** Panels on the ring; the rest stack behind it. Default 12. */
    maxPanels?: number;
    /** Each tier of stacked panels is this many metres further back. Default 0.25. */
    stackStep?: number;
    /** The triggers of the stock bindings by id; `null` leaves a binding out. */
    triggers?: {
        move?: string | null;
        resize?: string | null;
        rotate?: string | null;
    };
}
/**
 * Places room panels on a ring around the viewer and provides bindings to move, resize and rotate them. `bindings`
 * is the layout's collection: remove a binding to turn it off, or add another binding in its place.
 */
export declare class SpatialLayout {
    /** The collection of bindings that move, resize and rotate room panels. */
    readonly bindings: BindingCollection;
    /** Ring dimensions used to place panels, in millimetres and radians. */
    readonly ring: XRRingParams;
    constructor(options?: SpatialLayoutOptions);
    /** Places views around the ring and returns their poses and drawing order. */
    arrange<I>(views: readonly RingView<I>[], focus: I | null): RingResult;
}
