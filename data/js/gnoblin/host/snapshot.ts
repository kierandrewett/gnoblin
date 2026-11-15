// What a reload keeps: which window was on which workspace, which workspace each output showed, and the rectangles
// of windows a layout was not placing. `snapshotWorkspaces` writes it when a generation ends and
// `reconcileWorkspaces` puts it back in the next.

import { insetFromNumbers, insetToNumbers } from "gnoblin/frame/inset";
import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import type { KeptWindow, WorkspaceSnapshot } from "gnoblin/host/contract";
import { deskOutputs, primaryDesk, setShown } from "gnoblin/host/desks";
import type { Desk } from "gnoblin/host/desks";
import { moveClient } from "gnoblin/host/membership";
import { stateOf } from "gnoblin/host/node_state";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import { restoreSnap, snapRecord } from "gnoblin/snap";
import type { SnapZone } from "gnoblin/snap";
import type { Rect } from "gnoblin/types";
import { windowState } from "gnoblin/window_state";

/** Encodes a rectangle as x, y, width, height for a generation snapshot. */
const asArray = (rect: Rect | null | undefined): number[] | null =>
    rect ? [rect.x, rect.y, rect.width, rect.height] : null;

/** Restores a four-number rectangle from a generation snapshot. */
const asRect = (numbers: number[] | null | undefined): Rect | null =>
    numbers && numbers.length === 4 ? { x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3] } : null;

/** The rectangles each window with one keeps, and the frame inset it was fitted to, for the next generation. */
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

/** Gives the windows of the last generation the rectangles they were keeping. */
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

/** Which window is on which workspace, for the next generation to reconcile against. */
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
 * Puts the windows back on the workspaces of the same id. A window whose workspace this generation does not
 * declare moves to the first workspace; one that stays is not offered to the layout as an arrival, so a
 * floating window keeps the place the user gave it.
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
