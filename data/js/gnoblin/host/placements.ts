// Applies layout placements and slides by writing window bounds, visibility, depth and transforms.

import type { Client } from "gnoblin/host/client";
import { growRect } from "gnoblin/frame/inset";
import { ctx } from "gnoblin/host/context";
import { isShown } from "gnoblin/host/desks";
import { stateOf, writeProperty } from "gnoblin/host/node_state";
import { framed, workAreaOfWorkspace } from "gnoblin/host/work_area";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import { raiseSignal } from "gnoblin/properties";
import type { Triple } from "gnoblin/spatial";
import type { Rect } from "gnoblin/types";
import type { CheckedPlacement } from "gnoblin/workspace";

/** Writes only the rectangle properties that differ from `client`'s current values. */
function writeBounds(client: Client, rect: Rect): void {
    for (const key of ["x", "y", "width", "height"] as const) {
        if (client[key] !== rect[key]) {
            writeProperty(client, key, rect[key], "number");
        }
    }
}

/** Checks whether two rotation or scale vectors have the same components. */
function sameVector(a: Triple, b: Triple): boolean {
    return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

/** Records the intended visibility and writes it only when the value changes. */
function writeShown(client: Client, shown: boolean): void {
    ctx.writtenVisible.set(client, shown);
    if (client.visible !== shown) {
        client.visible = shown;
    }
}

/**
 * The rectangle a placement gives `client`: its bounds moved by the offset of its transform's position, to whole
 * pixels. With no bounds the rectangle is the window's own size (frame included) centred in `area`.
 */
function placementBounds(client: Client, placement: CheckedPlacement<Client>, area: Rect): Rect | null {
    const { bounds, offset } = placement;
    if (offset === null) {
        return bounds;
    }
    const room = growRect({ x: 0, y: 0, width: client.width, height: client.height }, client.frameInset);
    const base = bounds ?? {
        x: area.x + (area.width - room.width) / 2,
        y: area.y + (area.height - room.height) / 2,
        width: room.width,
        height: room.height,
    };
    return { ...base, x: Math.round(base.x + offset.x), y: Math.round(base.y + offset.y) };
}

/**
 * Writes what a layout's placements say beyond the rectangle: whether the window is tiled, hidden, and where it is in
 * depth. A client the layout gave no placement is as if placed with the defaults, so a window that leaves a tiled
 * layout for a floating one stops being tiled. Only what differs from the client's value is written. A placement
 * whose `transform` has a position moves the rectangle by that offset: from `bounds` when it has them, else from the
 * rectangle of the window's own size centred in `area`.
 */
export function applyPlacements(
    children: readonly Client[],
    placements: readonly CheckedPlacement<Client>[],
    area: Rect,
): void {
    const placed = new Map(placements.map((placement) => [placement.node, placement]));
    for (const client of children) {
        const placement = placed.get(client);
        const bounds = placement ? placementBounds(client, placement, area) : null;
        if (bounds) {
            writeBounds(client, framed(client, bounds));
        }
        const tiled = placement?.tiled ?? false;
        if (client.tiled !== tiled) {
            client.tiled = tiled;
        }
        const wanted = placement?.visible ?? true;
        if ((ctx.shownByLayout.get(client) ?? true) !== wanted) {
            raiseSignal(client, "shown");
        }
        ctx.shownByLayout.set(client, wanted);
        const z = placement?.z ?? 0;
        if (client.z !== z) {
            client.z = z;
        }
        const rotation = placement?.rotation ?? [0, 0, 0];
        if (!sameVector(client.rotation, rotation)) {
            client.rotation = rotation;
        }
        const scale = placement?.scale ?? [1, 1, 1];
        if (!sameVector(client.scale, scale)) {
            client.scale = scale;
        }
    }
}

/**
 * Shows the windows of the shown workspace (and of a workspace a slide is leaving) that their layout shows, and hides
 * the rest. A window is written only when what it should be differs from what this module last wrote.
 */
export function syncVisibility(): void {
    for (const workspace of ctx.workspaceRoot.list as Workspace[]) {
        const shown = isShown(workspace) || ctx.holding.has(workspace);
        for (const client of wsState(workspace).clientsState.list as Client[]) {
            const want = shown && (ctx.shownByLayout.get(client) ?? true);
            if (client.alive && (ctx.writtenVisible.get(client) ?? true) !== want) {
                writeShown(client, want);
            }
        }
    }
}

/**
 * Starts the slides `show` asked for, once the layout has put every window at rest: the windows of the workspace
 * that comes in start a screen's width away on the side it comes from and move to where the layout put them, and
 * the windows of the one that leaves move a screen's width the other way. The leaving windows stay drawn until
 * the slide ends, then go back to where they rest, hidden.
 */
export function startSlides(): void {
    for (const slide of ctx.slides.splice(0)) {
        const { from, to, direction, duration, progress } = slide;
        // The slide is as long as the output the incoming workspace is on is wide.
        const area = workAreaOfWorkspace(to);
        if (area === null) {
            continue;
        }
        const travel = direction * area.width;
        const moves: { node: number; key: string; value: number }[] = [];
        for (const client of wsState(to).clientsState.list as Client[]) {
            const rest = client.x;
            client.x = rest + travel * (1 - progress);
            moves.push({ node: stateOf(client).node, key: "x", value: rest });
        }
        const resting = new Map<Client, number>();
        for (const client of wsState(from).clientsState.list as Client[]) {
            const rest = client.x;
            resting.set(client, rest);
            if (progress > 0) {
                client.x = rest - travel * progress;
            }
            moves.push({ node: stateOf(client).node, key: "x", value: rest - travel });
        }
        ctx.transitions.play(moves, { duration, easing: "ease-out" });
        ctx.holding.set(from, (ctx.holding.get(from) ?? 0) + 1);
        ctx.hostApi.timers.after(duration, () => {
            const left = (ctx.holding.get(from) ?? 1) - 1;
            if (left > 0) {
                ctx.holding.set(from, left);
                return;
            }
            ctx.holding.delete(from);
            for (const [client, rest] of resting) {
                if (client.alive && ctx.membership.get(client) === from && !isShown(from)) {
                    writeShown(client, false);
                    client.x = rest;
                }
            }
        });
    }
}
