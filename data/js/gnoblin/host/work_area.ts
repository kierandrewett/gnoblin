// Rectangles: an output's, the area a layout tiles in on it (the output less its bars), and which of those a
// client is in.

import { shrinkRect } from "gnoblin/frame/inset";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { deskOfClient, deskShowing, primaryOutput } from "gnoblin/host/desks";
import type { Desk } from "gnoblin/host/desks";
import { stateOf } from "gnoblin/host/node_state";
import type { Output } from "gnoblin/host/output";
import type { Workspace } from "gnoblin/host/workspace";
import type { Rect } from "gnoblin/types";
import { barInsets } from "gnoblin/widgets";

/** An output's rectangle in the compositor's coordinates. Null for a gone output or one with no size yet. */
function rectOf(output: Output): Rect | null {
    const state = stateOf(output);
    if (!state.alive) {
        return null;
    }
    const width = host.get(state.node, "width");
    const height = host.get(state.node, "height");
    if (typeof width !== "number" || typeof height !== "number" || !(width > 0) || !(height > 0)) {
        return null;
    }
    const x = host.get(state.node, "x");
    const y = host.get(state.node, "y");
    return { x: typeof x === "number" ? x : 0, y: typeof y === "number" ? y : 0, width, height };
}

/** What a layout tiles in on `output`: its rectangle less the bars on it. */
export function areaOf(output: Output): Rect | null {
    const rect = rectOf(output);
    if (rect === null) {
        return null;
    }
    const insets = barInsets(output);
    return {
        x: rect.x,
        y: rect.y + insets.top,
        width: rect.width,
        height: Math.max(1, rect.height - insets.top - insets.bottom),
    };
}

/**
 * The one area a spanning desk's workspace is laid out in: the bounding box of the work areas of the outputs it covers.
 * A union of unequal outputs has corners no output shows; a window a layout puts there is not visible.
 */
function spanArea(outputs: readonly Output[]): Rect | null {
    const areas = outputs.map(areaOf).filter((area): area is Rect => area !== null);
    if (areas.length === 0) {
        return null;
    }
    const left = Math.min(...areas.map((area) => area.x));
    const top = Math.min(...areas.map((area) => area.y));
    const right = Math.max(...areas.map((area) => area.x + area.width));
    const bottom = Math.max(...areas.map((area) => area.y + area.height));
    return { x: left, y: top, width: right - left, height: bottom - top };
}

/** The area of the output a desk shows on: null for a desk with no output, a parked one, or an output with no size. */
export function areaOfDesk(desk: Desk): Rect | null {
    if (desk.output === null || desk.parked) {
        return null;
    }
    return desk.spans === null ? areaOf(desk.output) : spanArea(desk.spans);
}

/** The output of `desk` that holds the middle of `client`: a spanning desk covers several, any other has one. */
function deskOutputAt(desk: Desk, client: Client | null): Output | null {
    if (desk.output === null || desk.parked) {
        return null;
    }
    if (desk.spans === null || client === null) {
        return desk.output;
    }
    const x = client.x + client.width / 2;
    const y = client.y + client.height / 2;
    return (
        desk.spans.find((output) => {
            const rect = rectOf(output);
            return rect !== null && x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height;
        }) ?? desk.output
    );
}

/** The primary output as a rectangle: the whole output. Null when the tree has no output yet. */
function screenRect(): Rect | null {
    const output = primaryOutput();
    return output === null ? null : rectOf(output);
}

/** The area windows are laid out in on the primary output. Null when the tree has no output yet. */
export function screenArea(): Rect | null {
    const output = primaryOutput();
    return output === null ? null : areaOf(output);
}

/** The output rectangle `client` is on, or the primary output's when its workspace is on none. */
export function outputRectOf(client: Client | null): Rect | null {
    const desk = client === null ? null : deskOfClient(client);
    const output = desk === null ? null : deskOutputAt(desk, client);
    return output === null ? screenRect() : rectOf(output);
}

/** The work area of the output `client` is on, or the primary output's when its workspace is on none. */
export function workAreaOf(client: Client | null): Rect | null {
    const desk = client === null ? null : deskOfClient(client);
    const output = desk === null ? null : deskOutputAt(desk, client);
    return output === null ? screenArea() : areaOf(output);
}

/** The work area of the output that shows `workspace` (all of a span), or the primary output's when none does. */
export function workAreaOfWorkspace(workspace: Workspace): Rect | null {
    const desk = deskShowing(workspace);
    return desk?.output && !desk.parked ? areaOfDesk(desk) : screenArea();
}

/**
 * The content's rectangle for a frame: the window's frame lies round the content, so the content is the frame less the
 * frame's inset on each side. A client with no frame is the frame.
 */
export function framed(client: Client, frame: Rect): Rect {
    return shrinkRect(frame, client.frameInset);
}
