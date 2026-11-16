// Saves workspace membership and window placement across a reload.

import { insetFromNumbers, insetToNumbers } from "gnoblin-core/inset";
import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import type { KeptWindow, WorkspaceSnapshot } from "gnoblin-core/host/contract";
import { deskOutputs, primaryDesk, setShown } from "gnoblin-std/host/desks";
import type { Desk } from "gnoblin-std/host/desks";
import { moveClient } from "gnoblin-std/host/membership";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { restoreSnap, snapRecord } from "gnoblin-std/snap";
import type { SnapZone } from "gnoblin-std/snap";
import type { Rect } from "gnoblin-core/types";
import { windowState } from "gnoblin-std/window_state";

/** Converts a rectangle to x, y, width and height for a workspace snapshot. */
const asArray = (rect: Rect | null | undefined): number[] | null =>
    rect ? [rect.x, rect.y, rect.width, rect.height] : null;

/** Restores a rectangle from four snapshot numbers. */
const asRect = (numbers: number[] | null | undefined): Rect | null =>
    numbers && numbers.length === 4 ? { x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3] } : null;

/** Captures each window's saved rectangles and the frame inset used to fit it. */
function keptWindows(): KeptWindow[] {
    const kept: KeptWindow[] = [];
    for (const client of ctx.clientRoot.list as Client[]) {
        const rects = windowState.kept(client);
        const snap = snapRecord(client);
        const leftAt = (client as unknown as { floatingBounds?: Rect | null }).floatingBounds ?? null;
        // Keep a known zero too: a later generation must detect a frame appearing after one without a frame.
        const inset = ctx.fittedInset.get(client) ?? null;
        if (rects.floating === null && rects.windowed === null && snap === null && leftAt === null && inset === null) {
            continue;
        }
        kept.push({
            node: stateOf(client).node,
            floating: asArray(rects.floating),
            windowed: asArray(rects.windowed),
            leftAt: asArray(leftAt),
            zone: snap?.zone ?? null,
            snapFloating: asArray(snap?.floating),
            inset: inset === null ? null : insetToNumbers(inset),
        });
    }
    return kept;
}

/** Restores saved rectangles and snap state for windows from the previous generation. */
function restoreKept(kept: readonly KeptWindow[]): void {
    for (const entry of kept) {
        const client = ctx.clientsByNode.get(entry.node);
        if (client === undefined || !client.alive) {
            continue;
        }
        windowState.keep(client, { floating: asRect(entry.floating), windowed: asRect(entry.windowed) });
        const inset = insetFromNumbers(entry.inset);
        if (inset !== null) {
            ctx.fittedInset.set(client, inset);
        }
        const leftAt = asRect(entry.leftAt);
        if (leftAt !== null) {
            (client as unknown as { floatingBounds: Rect }).floatingBounds = leftAt;
        }
        const floating = asRect(entry.snapFloating);
        if (entry.zone !== null && floating !== null) {
            restoreSnap(client, { zone: entry.zone as SnapZone, floating });
        }
    }
}

/** Captures workspace membership and the workspace shown on each output. */
export function snapshotWorkspaces(): WorkspaceSnapshot {
    return {
        active: primaryDesk()?.shown?.id ?? null,
        workspaces: (ctx.workspaceRoot.list as Workspace[]).map((workspace) => ({
            id: workspace.id,
            clients: (wsState(workspace).clientsState.list as Client[]).map((client) => stateOf(client).node),
        })),
        outputs: deskOutputs().flatMap((output) => {
            const shown = ctx.deskByOutput.get(output)?.shown;
            return shown === null || shown === undefined ? [] : [{ output: output.name, workspace: shown.id }];
        }),
        kept: keptWindows(),
    };
}

/**
 * Restores window membership by workspace id. Unknown workspaces send their windows to the first workspace; windows
 * that remain on the same workspace are not marked as arrivals, so floating windows keep their positions.
 */
export function reconcileWorkspaces(previous: WorkspaceSnapshot): void {
    restoreKept(previous.kept ?? []);
    for (const entry of previous.workspaces) {
        const kept = (ctx.workspaceRoot.list as Workspace[]).find((workspace) => workspace.id === entry.id) ?? null;
        const target = kept ?? (ctx.workspaceRoot.list[0] as Workspace | undefined) ?? null;
        for (const node of entry.clients) {
            const client = ctx.clientsByNode.get(node);
            if (client) {
                moveClient(client, target, kept === null);
            }
        }
    }
    // Each output shows what it showed; the primary output's workspace is the one the snapshot calls active.
    const wanted: [Desk, Workspace][] = [];
    for (const entry of previous.outputs ?? []) {
        const output = deskOutputs().find((one) => one.name === entry.output);
        const workspace = (ctx.workspaceRoot.list as Workspace[]).find((one) => one.id === entry.workspace);
        const desk = output === undefined ? undefined : ctx.deskByOutput.get(output);
        if (desk !== undefined && workspace !== undefined) {
            wanted.push([desk, workspace]);
        }
    }
    const primary = primaryDesk();
    const shown = (ctx.workspaceRoot.list as Workspace[]).find((workspace) => workspace.id === previous.active);
    if (primary !== null && shown !== undefined && !wanted.some(([desk]) => desk === primary)) {
        wanted.push([primary, shown]);
    }
    for (const [desk, workspace] of wanted) {
        const other = ctx.desks.find((candidate) => candidate !== desk && candidate.shown === workspace);
        if (other !== undefined) {
            setShown(other, desk.shown);
        }
        setShown(desk, workspace);
    }
}
