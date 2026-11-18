// Saves workspace membership and window placement across a reload, and gives them back to the library of the next generation.
//
// A reload is a new generation with a library of its own, so what the old one held (which windows were on which workspace, which
// workspace each output showed) is read from it here and handed to the new one by workspace id.

import { insetFromNumbers, insetToNumbers } from "gnoblin-core/inset";
import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import type { KeptWindow, WorkspaceSnapshot } from "gnoblin-core/host/contract";
import { stateOf } from "gnoblin-core/host/node_state";
import { ClientFlags } from "gnoblin-std/generated/constants";
import { deskOutputs, primaryOutput, shownOn } from "gnoblin-std/host/desks";
import { nativeClient, nativeOutput } from "gnoblin-std/host/library";
import { syncMembership } from "gnoblin-std/host/membership";
import { showWorkspace } from "gnoblin-std/host/show";
import { std } from "gnoblin-std/host/state";
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

/** The room the library last fitted a window's frame to, or null while it has not fitted it. */
function fittedInset(client: Client): number[] | null {
    const state = nativeClient(client).state();
    return (state.flags & ClientFlags.HAS_FITTED) === 0
        ? null
        : insetToNumbers({
              top: state.fittedTop,
              right: state.fittedRight,
              bottom: state.fittedBottom,
              left: state.fittedLeft,
          });
}

/** Captures each window's saved rectangles and the frame inset used to fit it. */
function keptWindows(): KeptWindow[] {
    const kept: KeptWindow[] = [];
    for (const client of ctx.clientRoot.list as Client[]) {
        const rects = windowState.kept(client);
        const snap = snapRecord(client);
        const leftAt = (client as unknown as { floatingBounds?: Rect | null }).floatingBounds ?? null;
        // Keep a known zero too: a later generation must detect a frame appearing after one without a frame.
        const inset = fittedInset(client);
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
            inset,
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
            const native = nativeClient(client);
            const state = native.state();
            native.setState({
                ...state,
                flags: state.flags | ClientFlags.HAS_FITTED,
                fittedTop: inset.top,
                fittedRight: inset.right,
                fittedBottom: inset.bottom,
                fittedLeft: inset.left,
            });
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
    syncMembership();
    const workspaces = std.workspaceRoot.list as Workspace[];
    const primary = primaryOutput();
    const shown = primary === null ? (workspaces.find((workspace) => workspace.active) ?? null) : shownOn(primary);
    return {
        active: shown?.id ?? null,
        workspaces: workspaces.map((workspace) => ({
            id: workspace.id,
            clients: (wsState(workspace).clientsState.list as Client[]).map((client) => stateOf(client).node),
        })),
        outputs: deskOutputs().flatMap((output) => {
            const on = shownOn(output);
            return on === null ? [] : [{ output: output.name, workspace: on.id }];
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
    const workspaces = std.workspaceRoot.list as Workspace[];
    for (const entry of previous.workspaces) {
        const kept = workspaces.find((workspace) => workspace.id === entry.id) ?? null;
        const target = kept ?? workspaces[0] ?? null;
        for (const node of entry.clients) {
            if (target !== null && ctx.clientsByNode.has(node)) {
                wsState(target).native.restoreClient(node, kept === null ? 1 : 0);
            }
        }
    }
    syncMembership();
    // Each output shows what it showed; the primary output's workspace is the one the snapshot calls active.
    const wanted: [Workspace, number][] = [];
    for (const entry of previous.outputs ?? []) {
        const output = deskOutputs().find((one) => one.name === entry.output);
        const workspace = workspaces.find((one) => one.id === entry.workspace);
        if (output !== undefined && workspace !== undefined) {
            wanted.push([workspace, nativeOutput(output).node]);
        }
    }
    const primary = primaryOutput();
    const shown = workspaces.find((workspace) => workspace.id === previous.active);
    if (primary !== null && shown !== undefined && !wanted.some(([, node]) => node === nativeOutput(primary).node)) {
        wanted.push([shown, nativeOutput(primary).node]);
    }
    for (const [workspace, node] of wanted) {
        const output = deskOutputs().find((one) => nativeOutput(one).node === node);
        if (output !== undefined) {
            showWorkspace(workspace, { output });
        }
    }
}
