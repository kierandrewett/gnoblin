// Fitting a window to the work area it is in: after a workspace or output change, or when its titlebar comes
// or goes. A maximised window fills the area, a snapped one tiles to its zone again, a floating one shrinks to fit.

import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import { stateOf } from "gnoblin/host/node_state";
import { framed, outputRectOf } from "gnoblin/host/work_area";
import { restoreSnap, snapClient, snappedZone, snapRecord } from "gnoblin/snap";
import type { SnapClient } from "gnoblin/snap";
import type { Rect } from "gnoblin/types";
import { windowState } from "gnoblin/window_state";

/** A rectangle that moved to another output: shrunk to the area when larger, then moved inside it. */
function fitInto(rect: Rect, area: Rect): Rect {
    const width = Math.min(rect.width, area.width);
    const height = Math.min(rect.height, area.height);
    return {
        x: Math.min(Math.max(rect.x, area.x), area.x + area.width - width),
        y: Math.min(Math.max(rect.y, area.y), area.y + area.height - height),
        width,
        height,
    };
}

/**
 * The bar a window shows above its content, whether or not fullscreen hides it at the moment. `frameInset` is 0 for a
 * fullscreen window, but the room a window keeps for its bar is not: leaving fullscreen brings the bar back.
 */
function barHeight(client: Client): number {
    return ctx.decorations?.titlebarOf(client)?.style.height ?? 0;
}

/**
 * The content rectangle `rect` as a frame inside `area`: the content with `bar` on top, shrunk to fit and moved the least
 * distance that puts all of it inside. A bar is never left above the area's top edge.
 */
function fitContent(rect: Rect, bar: number, area: Rect): Rect {
    const frame = fitInto({ x: rect.x, y: rect.y - bar, width: rect.width, height: rect.height + bar }, area);
    return { x: frame.x, y: frame.y + bar, width: frame.width, height: Math.max(0, frame.height - bar) };
}

/** `area` less the room a bar takes along its top: where the content of a window that fills a frame in `area` goes. */
function belowBar(client: Client, area: Rect): Rect {
    const bar = barHeight(client);
    return { x: area.x, y: area.y + bar, width: area.width, height: Math.max(1, area.height - bar) };
}

/** The part of a client the rectangle it floats at is kept in. */
interface Floater {
    floatingBounds?: Rect | null;
}

/**
 * Fits a window to the work area it is now in: a maximised window fills it, a snapped one tiles to its zone again, a
 * floating one shrinks to fit and moves inside it (the frame, bar included, stays inside), and the rectangle it floats
 * at follows. A fullscreen window covers its output.
 */
export function refitClient(client: Client, area: Rect): void {
    if (client.fullscreen) {
        const output = outputRectOf(client);
        if (output !== null) {
            client.bounds = output;
        }
        return;
    }
    const zone = snappedZone(client);
    const bar = barHeight(client);
    if (client.maximised) {
        client.bounds = framed(client, area);
    } else if (zone !== null) {
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: belowBar(client, area) });
    } else {
        client.bounds = fitContent(client.bounds, bar, area);
    }
    const floater = client as unknown as Floater;
    if (floater.floatingBounds) {
        floater.floatingBounds = fitContent(floater.floatingBounds, bar, area);
    }
}

/**
 * A titlebar came or went (the compositor changed who draws the window's frame, or a reload toggled decorations): the
 * window's content does not move by itself, so a bar that mounts above a window placed at the top of the area would be
 * drawn off the screen. The rule is that the FRAME keeps its place: the content moves down by the bar that came, up by the
 * bar that went, and the result is clamped so the frame stays inside the work area. A maximised window fills the area
 * again, a snapped one tiles to its zone again, and a window a tiling layout places is placed again by the layout (the
 * decoration change marks its workspace dirty). The rectangles a window returns to (leaving maximised, unsnapping, the
 * one it floats at) move the same way, so they do not bring the old offset back. A fullscreen window is left alone and
 * fitted when it leaves: the rectangle it returns to is the one the bar was fitted to before.
 */
export function followBar(client: Client, area: Rect): void {
    if (client.fullscreen) {
        return;
    }
    const bar = barHeight(client);
    const was = ctx.fittedBar.get(client);
    ctx.fittedBar.set(client, bar);
    if (was === undefined || was === bar) {
        return;
    }
    const delta = bar - was;
    const moved = (rect: Rect): Rect => fitContent({ ...rect, y: rect.y + delta }, bar, area);
    const kept = windowState.kept(client).floating;
    if (kept !== null) {
        windowState.keep(client, { floating: moved(kept) });
    }
    const record = snapRecord(client);
    if (record !== null) {
        restoreSnap(client, { zone: record.zone, floating: moved(record.floating) });
    }
    const floater = client as unknown as Floater;
    if (floater.floatingBounds) {
        floater.floatingBounds = moved(floater.floatingBounds);
    }
    const zone = snappedZone(client);
    if (client.maximised) {
        client.bounds = framed(client, area);
    } else if (zone !== null) {
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: belowBar(client, area) });
    } else if (!client.tiled || stateOf(client).floating) {
        client.bounds = moved(client.bounds);
    }
}
