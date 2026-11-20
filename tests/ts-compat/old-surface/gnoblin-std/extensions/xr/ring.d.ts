/** Dimensions and spacing for a panel ring, in millimetres and radians. */
export interface XRRingParams {
    /** The distance from the viewer to the centre of a panel. */
    radius: number;
    /** The height of a panel's centre above the room's floor. */
    eye: number;
    /** One window pixel is this many millimetres. */
    mmPerPx: number;
    /** The focused panel is this much larger ... */
    focusScale: number;
    /** ... and this much closer to the viewer. */
    focusPull: number;
    /** The angle between neighbouring panels. */
    gap: number;
    /** The most the panels reach to either side of the front: panels shrink to fit. */
    halfArc: number;
    /** Panels on the ring; the rest stack behind it. */
    maxPanels: number;
    /** Each tier of stacked panels is this much further back. */
    stackStep: number;
}
/** Default ring dimensions in millimetres and radians. */
export declare const RING_DEFAULTS: Readonly<XRRingParams>;
/** The id and pixel size used to place one panel on the ring. */
export interface RingView<I> {
    /** Identifier compared with `focus`. */
    readonly id: I;
    /** Window width, in pixels. */
    readonly width: number;
    /** Window height, in pixels. */
    readonly height: number;
}
/** A panel's position, orientation and depth in the room. */
export interface RingPose {
    /** Horizontal position in millimetres. */
    x: number;
    /** Height in millimetres. */
    y: number;
    /** Depth in millimetres. */
    z: number;
    /** Rotation about each axis in radians; only the turn about y is used. */
    rotation: [number, number, number];
    /** Millimetres per pixel for the panel. */
    scale: number;
    /** Whether this panel is focused. */
    focused: boolean;
    /** 0 on the ring and the tier (1, 2, ...) of a stacked panel. */
    behind: number;
}
/** Ring poses in input order and their far-to-near rendering order. */
export interface RingResult {
    /** One pose per view, in the order of the views. */
    poses: RingPose[];
    /** The view indices from the farthest to the nearest: the order the room must list the Views in. */
    order: number[];
}
/**
 * Places panels around a ring from their pixel sizes and the focused id. The focused panel is at the front, with
 * neighbours in creation order on either side (the right side holds the extra one), wrapping around; without a focus,
 * the first panel is at the front.
 */
export declare function arrangeRing<I>(views: readonly RingView<I>[], focus: I | null, params?: Readonly<XRRingParams>): RingResult;
