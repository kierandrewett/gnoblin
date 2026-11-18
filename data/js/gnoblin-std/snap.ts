// Edge tiling snaps a dragged window to the left or right half, or the full work area at the top edge. It is off
// unless the seat's `edge_tiling` is true. The behaviour follows the Lua shell's snap methods; coverage is in
// `crates/gnoblin-js/tests/stock_focus.rs`.
//
// The geometry helpers use plain rectangles. `snapClient` and `unsnapClient` work with objects that have `bounds`;
// the library keeps the snap of a window with its frame inset and the rectangle it floats at.

import type { Client } from "gnoblin-core/host/client";
import type { FrameInset } from "gnoblin-core/inset";
import type { Rect } from "gnoblin-core/types";
import { Drag } from "gnoblin-std/generated/classes";
import type { Client as NativeClient } from "gnoblin-std/generated/classes";
import { whenKnown } from "gnoblin-std/host/client_state";
import { nativeClient } from "gnoblin-std/host/library";
import { SnapKind } from "gnoblin-std/generated/constants";

/** The part of the work area a window tiles to: the left or right half, or all of it. */
export type SnapZone = "left" | "right" | "top";

/** How near an edge the pointer must be to name it, in pixels. A small fixed zone. */
export const SNAP_MARGIN = SnapKind.MARGIN;

/** The names of the zones, by the number the library knows them by. */
const ZONES: Readonly<Record<number, SnapZone>> = {
    [SnapKind.LEFT]: "left",
    [SnapKind.RIGHT]: "right",
    [SnapKind.TOP]: "top",
};

/** The number the library knows a zone by; none for a name that is not a zone. */
const ZONE_NUMBERS: Readonly<Record<string, number>> = {
    left: SnapKind.LEFT,
    right: SnapKind.RIGHT,
    top: SnapKind.TOP,
};

/** The zone the library's number names. */
export function zoneNamed(zone: number): SnapZone {
    const named = ZONES[zone];
    if (named === undefined) {
        throw new RangeError(`${zone} is not a snap zone`);
    }
    return named;
}

/** Whether edge tiling is on. Absent is off; anything but a boolean throws. */
export function edgeTilingEnabled(value: unknown): boolean {
    if (value === undefined) {
        return false;
    }
    if (typeof value !== "boolean") {
        throw new TypeError(`edge_tiling must be a boolean, got ${String(value)}`);
    }
    return value;
}

/** The edge the pointer is within `margin` pixels of, or null. Left and right win over top at corners. */
export function edgeZone(x: number, y: number, area: Rect, margin: number = SNAP_MARGIN): SnapZone | null {
    return ZONES[Drag.zone(x, y, area, margin)] ?? null;
}

/** The rectangle a window tiles to in `zone`, or null when the zone does not tile. The halves are whole pixels. */
export function tileRect(zone: string, area: Rect): Rect | null {
    return Object.hasOwn(ZONE_NUMBERS, zone) ? Drag.tile(ZONE_NUMBERS[zone], area) : null;
}

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

/** The library's record of a client: the snap lives with its frame inset and floating rectangle, where a refit reads it. */
function library(client: object): NativeClient {
    return nativeClient(client as unknown as Client);
}

/**
 * Tiles `client` to `zone`. Refused (false) when edge tiling is off or the zone does not tile, and the client stays
 * where it was. A maximised client floats first, so unsnapping returns it to the rectangle it had before. A client
 * that is snapped already keeps the rectangle it first floated at. The library tiles it, less the room its frame takes.
 */
export function snapClient(client: SnapClient, zone: string, options: SnapOptions): boolean {
    if (!Object.hasOwn(ZONE_NUMBERS, zone)) {
        return false;
    }
    return whenKnown(
        () => library(client).snap(ZONE_NUMBERS[zone], options.area, options.edgeTiling ? 1 : 0),
        () => false,
    );
}

/** Puts a snapped client back where it floated. False when it is not snapped. */
export function unsnapClient(client: SnapClient): boolean {
    return whenKnown(
        () => library(client).unsnap(0),
        () => false,
    );
}

/** A copy of what `client`'s snap keeps, or null when it is not snapped: what `restoreSnap` takes back. */
export function snapRecord(client: object): SnapRecord | null {
    return whenKnown(
        () => {
            const state = library(client).state();
            const zone = ZONES[state.snapZone];
            return zone === undefined ? null : { zone, floating: { ...state.snapFloating } };
        },
        () => null,
    );
}

/** Snaps `client` again from a record `snapRecord` took, without moving it: the caller put the rectangle back. */
export function restoreSnap(client: object, record: SnapRecord): void {
    library(client).restoreSnap(ZONE_NUMBERS[record.zone], record.floating);
}

/** The zone `client` is snapped to, or null. */
export function snappedZone(client: object): SnapZone | null {
    return snapRecord(client)?.zone ?? null;
}

/** Forgets the snap of `client` without moving it: the window left its zone some other way. */
export function forgetSnap(client: object): void {
    whenKnown(
        () => library(client).unsnap(1),
        () => false,
    );
}
