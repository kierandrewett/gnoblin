// Fitting a window to the work area it is in: after a workspace or output change, or when its frame comes
// or goes, or changes. A maximised window fills the area, a snapped one tiles to its zone again, a floating one shrinks to fit.

import { growRect, insetArea, NO_INSET, sameInset, shrinkRect } from "gnoblin/frame/inset";
import type { FrameInset } from "gnoblin/frame/inset";
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
 * The room a window's frame keeps round its content, whether or not fullscreen hides the frame at the moment.
 * `frameInset` is 0 on every side for a fullscreen window, but the room a window keeps for its frame is not: leaving
 * fullscreen brings the frame back.
 */
function reservedInset(client: Client): FrameInset {
    return ctx.decorations?.reservedOf(client) ?? NO_INSET;
}

/**
 * The content rectangle `rect` as a frame inside `area`: the content with `inset` around it, shrunk to fit and moved the
 * least distance that puts all of it inside. A frame is never left across the area's edge.
 */
function fitContent(rect: Rect, inset: FrameInset, area: Rect): Rect {
    return shrinkRect(fitInto(growRect(rect, inset), area), inset);
}

/** `area` less the room a frame takes: where the content of a window that fills a frame in `area` goes. */
function insideFrame(client: Client, area: Rect): Rect {
    return insetArea(area, reservedInset(client));
}

/** The part of a client the rectangle it floats at is kept in. */
interface Floater {
    floatingBounds?: Rect | null;
}

/**
 * Fits a window to the work area it is now in: a maximised window fills it, a snapped one tiles to its zone again, a
 * floating one shrinks to fit and moves inside it (the frame, outlets included, stays inside), and the rectangle it floats
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
    const inset = reservedInset(client);
    if (client.maximised) {
        client.bounds = framed(client, area);
    } else if (zone !== null) {
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: insideFrame(client, area) });
    } else {
        client.bounds = fitContent(client.bounds, inset, area);
    }
    const floater = client as unknown as Floater;
    if (floater.floatingBounds) {
        floater.floatingBounds = fitContent(floater.floatingBounds, inset, area);
    }
}

/**
 * A frame came, went or changed (the compositor changed who draws the window's frame, a reload toggled decorations, or a
 * script gave an outlet another size): the window's content does not move by itself, so an outlet that appears above a
 * window placed at the top of the area would be drawn off the screen. The rule is that the FRAME keeps its place: the
 * content moves in by what the top and left gained and out by what they lost, and the result is clamped so the frame stays
 * inside the work area. A maximised window fills the area again, a snapped one tiles to its zone again, and a window a
 * tiling layout places is placed again by the layout (the decoration change marks its workspace dirty). The rectangles a
 * window returns to (leaving maximised, unsnapping, the one it floats at) move the same way, so they do not bring the
 * old offset back. A fullscreen window is left alone and fitted when it leaves: the rectangle it returns to is the one the
 * frame was fitted to before. Returns whether the frame changed since the window was last fitted, so a tiling layout
 * places its windows again.
 */
export function followFrame(client: Client, area: Rect): boolean {
    if (client.fullscreen) {
        return false;
    }
    const inset = reservedInset(client);
    const was = ctx.fittedInset.get(client);
    ctx.fittedInset.set(client, inset);
    if (was === undefined || sameInset(was, inset)) {
        return false;
    }
    // The frame keeps its place: the content moves in by what the top and left gained.
    const dx = inset.left - was.left;
    const dy = inset.top - was.top;
    const moved = (rect: Rect): Rect => fitContent({ ...rect, x: rect.x + dx, y: rect.y + dy }, inset, area);
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
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: insideFrame(client, area) });
    } else if (!client.tiled || stateOf(client).floating) {
        client.bounds = moved(client.bounds);
    }
    return true;
}
