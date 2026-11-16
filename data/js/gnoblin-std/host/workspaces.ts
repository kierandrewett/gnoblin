// Adds and removes script workspaces and assigns them to windows.

import { GnoblinError } from "gnoblin-core/errors";
import type { Client } from "gnoblin-core/host/client";
import { holders, recompute } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { currentDesk, deskShowing, ensureDesk, fillDesks, setShown } from "gnoblin-std/host/desks";
import { queueFor } from "gnoblin-core/host/dispatch";
import { moveClient, placeDialog } from "gnoblin-std/host/membership";
import { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { disposeBindingsOf, raiseSignal } from "gnoblin-core/properties";
import { initialWorkspace } from "gnoblin-std/workspace";

/** The workspace a window that has none joins: the one the output under the pointer shows, else the initial rule's. */
export function newWindowWorkspace(): Workspace | null {
    return currentDesk()?.shown ?? initialWorkspace(ctx.workspaceRoot.list as Workspace[]);
}

/** Gives each window that has no workspace the one the initial-workspace rule names. */
export function adoptOrphans(): void {
    if (!ctx.flags.opened) {
        return;
    }
    for (const client of [...ctx.clientRoot.list] as Client[]) {
        if (ctx.membership.has(client)) {
            continue;
        }
        if (placeDialog(client)) {
            continue;
        }
        const target = newWindowWorkspace();
        if (target === null) {
            return;
        }
        moveClient(client, target, !ctx.preexisting.has(client));
    }
}

/** Attaches a workspace to the public root and adopts any clients that are waiting for one. */
export function addWorkspace(workspace: Workspace): void {
    const state = wsState(workspace);
    if (state.attached) {
        return;
    }
    if (ctx.workspaceRoot.list.some((other) => other.id === state.id)) {
        throw new GnoblinError(
            "already_exists",
            "gnoblin.workspaces.add",
            `there is already a workspace with id "${state.id}"`,
        );
    }
    state.attached = true;
    state.dirty = true;
    ctx.workspaceRoot.list.push(workspace);
    raiseSignal(ctx.workspaceRoot, "items");
    queueFor(ctx.workspaceRoot, "onAdded", [workspace, ctx.workspaceRoot.self]);
    recompute(ctx.workspaceRoot, workspace);
    ensureDesk();
    fillDesks();
    ctx.displayManager.changed();
    adoptOrphans();
}

/**
 * Takes the workspace out of `gnoblin.workspaces`. Its windows move to the neighbour before it (the one after it,
 * for the first), and that neighbour is shown where the removed one was; none is closed.
 */
export function removeWorkspace(workspace: Workspace): void {
    ctx.displayManager.changed();
    const state = wsState(workspace);
    if (!state.attached) {
        return;
    }
    const held = holders(ctx.workspaceRoot, workspace);
    state.attached = false;
    const at = ctx.workspaceRoot.list.indexOf(workspace);
    const neighbour = (ctx.workspaceRoot.list[at - 1] ?? ctx.workspaceRoot.list[at + 1]) as Workspace | undefined;
    ctx.workspaceRoot.list.splice(at, 1);
    raiseSignal(ctx.workspaceRoot, "items");
    disposeBindingsOf(workspace);
    for (const view of held) {
        view.members?.delete(workspace);
        queueFor(view, "onRemoved", [workspace, view.self]);
    }
    const target = neighbour ?? null;
    for (const desk of ctx.desks) {
        if (desk.shown === workspace) {
            desk.shown = null;
            // The neighbour takes its place, unless another output shows it already: then the first workspace that
            // no output shows.
            const free =
                neighbour !== undefined && deskShowing(neighbour) === null
                    ? neighbour
                    : ((ctx.workspaceRoot.list as Workspace[]).find((one) => deskShowing(one) === null) ?? null);
            setShown(desk, free);
        }
    }
    for (const client of [...state.clientsState.list] as Client[]) {
        moveClient(client, target, true);
    }
}

/**
 * A script that declared no workspace gets one floating `desktop`, so there is always somewhere for a window to go. The
 * stock default's four workspaces are declared by `data/js/starter/init.ts`; a user's `init.ts` that declares none gets this one.
 */
export function ensureDefaultWorkspace(): void {
    if (ctx.workspaceRoot.list.length > 0) {
        return;
    }
    addWorkspace(new Workspace({ id: "desktop" }));
}
