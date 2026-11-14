// Edge tiling: dropping a dragged window on a screen edge tiles it to that part of the work area. Off unless the
// seat's `edge_tiling` is true. Ported from data/lua/gnoblin/shell/snap.lua (and `Window:snap` / `Window:unsnap` of
// data/lua/gnoblin/shell/init.lua); the Lua tests of both are the cases tests/stock_focus.rs runs.
//
// The pure functions take plain rectangles. `snapClient` and `unsnapClient` act on anything with `bounds`, which is
// a `Client`, and keep what they need to undo a snap off the client, so a library that does not own the object
// leaves nothing on it.

import type { Rect } from "gnoblin/layout";

/** The part of the work area a window tiles to: the left or right half, or all of it. */
export type SnapZone = "left" | "right" | "top";

/** How near an edge the pointer must be to name it, in pixels. Lua's `EDGE_MARGIN`: a small fixed zone. */
export const SNAP_MARGIN = 4;

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
    if (x - area.x <= margin) {
        return "left";
    }
    if (area.x + area.width - x <= margin) {
        return "right";
    }
    if (y - area.y <= margin) {
        return "top";
    }
    return null;
}

/** The rectangle a window tiles to in `zone`, or null when the zone does not tile. The halves are whole pixels. */
export function tileRect(zone: string, area: Rect): Rect | null {
    const half = Math.floor(area.width / 2);
    switch (zone) {
        case "left":
            return { x: area.x, y: area.y, width: half, height: area.height };
        case "right":
            return { x: area.x + half, y: area.y, width: area.width - half, height: area.height };
        case "top":
            return { x: area.x, y: area.y, width: area.width, height: area.height };
        default:
            return null;
    }
}

/** What `snapClient` reads and writes of a client. */
export interface SnapClient {
    bounds: Rect;
    /** Written `false` before a maximised client snaps: it floats first. */
    maximised?: boolean;
    /** The rectangle a maximised client had before it was maximised, when the shell kept one. */
    floatingBounds?: Rect | null;
}

interface Snapped {
    zone: SnapZone;
    /** The rectangle `unsnapClient` restores. */
    floating: Rect;
}

const snapped = new WeakMap<object, Snapped>();

/** What `snapClient` needs of the world: whether edge tiling is on, and the area a window tiles in. */
export interface SnapOptions {
    edgeTiling: boolean;
    area: Rect;
}

/**
 * Tiles `client` to `zone`. Refused (false) when edge tiling is off or the zone does not tile, and the client stays
 * where it was. A maximised client floats first, so unsnapping returns it to the rectangle it had before. A client
 * that is snapped already keeps the rectangle it first floated at.
 */
export function snapClient(client: SnapClient, zone: string, options: SnapOptions): boolean {
    const rect = options.edgeTiling ? tileRect(zone, options.area) : null;
    if (rect === null) {
        return false;
    }
    const before = snapped.get(client);
    let floating: Rect;
    if (before !== undefined) {
        floating = before.floating;
    } else if (client.maximised === true) {
        floating = { ...(client.floatingBounds ?? client.bounds) };
    } else {
        floating = { ...client.bounds };
    }
    if (client.maximised === true) {
        client.maximised = false;
    }
    snapped.set(client, { zone: zone as SnapZone, floating });
    client.bounds = rect;
    return true;
}

/** Puts a snapped client back where it floated. False when it is not snapped. */
export function unsnapClient(client: SnapClient): boolean {
    const state = snapped.get(client);
    if (state === undefined) {
        return false;
    }
    snapped.delete(client);
    client.bounds = state.floating;
    return true;
}

/** The zone `client` is snapped to, or null. */
export function snappedZone(client: object): SnapZone | null {
    return snapped.get(client)?.zone ?? null;
}

/** Ends the snapped state without moving the client, for a client the user is now dragging somewhere else. */
export function forgetSnap(client: object): void {
    snapped.delete(client);
}
