import type { FrameInset } from "gnoblin-core/inset";
import type { Rect } from "gnoblin-core/types";
/** The part of the work area a window tiles to: the left or right half, or all of it. */
export type SnapZone = "left" | "right" | "top";
/** How near an edge the pointer must be to name it, in pixels. A small fixed zone. */
export declare const SNAP_MARGIN: 4;
/** The zone the library's number names. */
export declare function zoneNamed(zone: number): SnapZone;
/** Whether edge tiling is on. Absent is off; anything but a boolean throws. */
export declare function edgeTilingEnabled(value: unknown): boolean;
/** The edge the pointer is within `margin` pixels of, or null. Left and right win over top at corners. */
export declare function edgeZone(x: number, y: number, area: Rect, margin?: number): SnapZone | null;
/** The rectangle a window tiles to in `zone`, or null when the zone does not tile. The halves are whole pixels. */
export declare function tileRect(zone: string, area: Rect): Rect | null;
/** What `snapClient` reads and writes of a client. */
export interface SnapClient {
    /** Current content rectangle in logical pixels. */
    bounds: Rect;
    /** Written `false` before a maximised client snaps: it floats first. */
    maximised?: boolean;
    /** The rectangle a maximised client had before it was maximised, when the shell kept one. */
    floatingBounds?: Rect | null;
}
/** What a snap keeps: the zone and the rectangle unsnapping goes back to. */
export interface SnapRecord {
    /** Edge region the client was snapped to. */
    zone: SnapZone;
    /** The rectangle `unsnapClient` restores. */
    floating: Rect;
}
/**
 * What `snapClient` needs of the world: whether edge tiling is on, the area a window tiles in, and the room the
 * window's frame takes round its content. The zone is a part of `area` for the FRAME: the content is the zone less
 * `inset` on every side, so a window with a frame on its left and right still fills the whole half.
 */
export interface SnapOptions {
    /** Whether edge tiling is enabled for this operation. */
    edgeTiling: boolean;
    /** Rectangle available for tiling, in logical pixels. */
    area: Rect;
    /** The frame's room round the content. Left out, the window has no frame. */
    inset?: FrameInset;
}
/**
 * What the shell host gives snapping for the windows it holds: the library keeps the snap of a window with its frame inset and the
 * rectangle it floats at, where a refit reads it. An object that is not a window (the pure helpers' tests) keeps its own record.
 */
export interface SnapHost {
    /** Whether `client` is a window the library holds. */
    owns(client: object): boolean;
    /** The library tiles the window to the zone number, less the room its frame takes. */
    snap(client: object, zone: number, area: Rect, edgeTiling: boolean): boolean;
    /** Puts a snapped window back where it floated, or only forgets the snap with `forget`. */
    unsnap(client: object, forget: boolean): boolean;
    /** The snap the library keeps for the window, or null. */
    record(client: object): {
        zone: number;
        floating: Rect;
    } | null;
    /** Gives the library back the snap a reload took. */
    restore(client: object, zone: number, floating: Rect): void;
}
/** Gives snapping its host. The package install calls it once per generation. */
export declare function attachSnap(host: SnapHost): void;
/**
 * Tiles `client` to `zone`. Refused (false) when edge tiling is off or the zone does not tile, and the client stays
 * where it was. A maximised client floats first, so unsnapping returns it to the rectangle it had before. A client
 * that is snapped already keeps the rectangle it first floated at. The library tiles a window, less the room its frame takes.
 */
export declare function snapClient(client: SnapClient, zone: string, options: SnapOptions): boolean;
/** Puts a snapped client back where it floated. False when it is not snapped. */
export declare function unsnapClient(client: SnapClient): boolean;
/** A copy of what `client`'s snap keeps, or null when it is not snapped: what `restoreSnap` takes back. */
export declare function snapRecord(client: object): SnapRecord | null;
/** Snaps `client` again from a record `snapRecord` took, without moving it: the caller put the rectangle back. */
export declare function restoreSnap(client: object, record: SnapRecord): void;
/** The zone `client` is snapped to, or null. */
export declare function snappedZone(client: object): SnapZone | null;
/** Forgets the snap of `client` without moving it: the window left its zone some other way. */
export declare function forgetSnap(client: object): void;
