// Tracks workspace membership, dialog placement and client transfers.

import { GnoblinError } from "gnoblin-core/errors";
import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { recompute } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { isShown } from "gnoblin-std/host/desks";
import { guard, queueFor } from "gnoblin-core/host/dispatch";
import { refitClient } from "gnoblin-std/host/fitting";
import { isClient, stateOf } from "gnoblin-core/host/node_state";
import { hasFocus } from "gnoblin-core/host/seat";
import { workAreaOfWorkspace } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { isWorkspace, wsState } from "gnoblin-std/host/workspace_state";
import type { WorkspaceState } from "gnoblin-std/host/workspace_state";
import { raiseSignal } from "gnoblin-core/properties";

/** A workspace's clients traded places: its layout computes again, and one with a record of its own trades them
 * there.
 */
export function swapClients(workspace: Workspace, a: Client, b: Client): void {
    const state = wsState(workspace);
    state.dirty = true;
    guard(`${workspace}.layout.onClientsSwapped`, () => state.layout.onClientsSwapped?.(a, b));
}

/** Puts `client` in `workspace`, which it is not in. */
function link(workspace: Workspace, client: Client, arrived: boolean): void {
    const state = wsState(workspace);
    ctx.membership.set(client, workspace);
    state.clientsState.list.push(client);
    raiseSignal(state.clientsState, "items");
    state.dirty = true;
    if (arrived) {
        state.arrived.push(client);
        ctx.offered.add(client);
    }
    queueFor(state.clientsState, "onAdded", [client, state.clientsState.self]);
    recompute(state.clientsState, client);
}

/** Takes `client`, which is alive, out of `workspace`. */
function unlink(workspace: Workspace, client: Client): void {
    const state = wsState(workspace);
    ctx.membership.delete(client);
    state.clientsState.list.splice(state.clientsState.list.indexOf(client), 1);
    raiseSignal(state.clientsState, "items");
    state.dirty = true;
    forgetArrival(state, client);
    queueFor(state.clientsState, "onRemoved", [client, state.clientsState.self]);
    recompute(state.clientsState, client);
}

/** A client that leaves before its layout heard of it never was there for the layout. */
export function forgetArrival(state: WorkspaceState, client: Client): void {
    const at = state.arrived.indexOf(client);
    if (at >= 0) {
        state.arrived.splice(at, 1);
    } else if (ctx.offered.has(client) || ctx.preexisting.has(client)) {
        state.departed.push(client);
    }
}

/** Moves `client` to `target` (or out of every workspace when it is null). `arrived` says a layout may place it. */
export function moveClient(client: Client, target: Workspace | null, arrived: boolean): void {
    const from = ctx.membership.get(client) ?? null;
    if (from === target) {
        return;
    }
    if (from) {
        unlink(from, client);
    }
    if (target) {
        link(target, client, arrived);
    }
    // A dialog stays with its parent: it goes wherever the parent goes.
    for (const dialog of dialogsOf(client)) {
        moveClient(dialog, target, false);
    }
}

/** The live clients that are dialogs of `parent`. */
export function dialogsOf(parent: Client): Client[] {
    return (ctx.clientRoot.list as Client[]).filter((client) => client.alive && client.parentWindow === parent);
}

/**
 * Puts a dialog where its parent is, and centres it in the parent the first time. Returns whether it did: a dialog
 * whose parent is on no workspace (or is gone) is placed as any other window.
 */
export function placeDialog(client: Client): boolean {
    const parent = client.parentWindow;
    const workspace = parent === null ? undefined : ctx.membership.get(parent);
    if (parent === null || workspace === undefined) {
        return false;
    }
    if (ctx.membership.get(client) !== workspace) {
        moveClient(client, workspace, false);
    }
    if (!ctx.centred.has(client)) {
        ctx.centred.add(client);
        client.x = parent.x + Math.floor((parent.width - client.width) / 2);
        client.y = parent.y + Math.floor((parent.height - client.height) / 2);
    }
    return true;
}

/** Hands the keyboard to the first window still showing on `workspace`, or to nobody. A tree with no seat has none. */
function passFocusFrom(workspace: Workspace | null, leaving: Client): void {
    if (host.input.seats().length === 0) {
        return;
    }
    const next =
        workspace === null
            ? undefined
            : (wsState(workspace).clientsState.list as Client[]).find(
                  (one) => one !== leaving && one.alive && !one.minimised,
              );
    if (next === undefined) {
        ctx.input.primary.clearFocus();
    } else {
        next.focus();
    }
}

/** `client.moveToWorkspace(workspace)`: the retired shell's `Window:move_to`. */
export function moveClientTo(client: Client, workspace: Workspace): boolean {
    if (!isWorkspace(workspace)) {
        throw new TypeError(`${client}.moveToWorkspace: needs a Workspace`);
    }
    const from = ctx.membership.get(client) ?? null;
    if (
        !client.alive ||
        !wsState(workspace).attached ||
        client.fullscreen ||
        client.parentWindow !== null ||
        from === workspace
    ) {
        return false;
    }
    const wasFocused = hasFocus(client);
    moveClient(client, workspace, true);
    // The window left the screen, so the focus passes to a window that stays.
    if (wasFocused && !isShown(workspace)) {
        passFocusFrom(from, client);
    }
    const area = workAreaOfWorkspace(workspace);
    if (area !== null) {
        refitClient(client, area);
    }
    return true;
}

/** `workspace.clients.add(client)`: the client leaves its workspace and joins this one. */
export function transferClient(workspace: Workspace, client: Client): void {
    const target = `${workspace}.clients.add`;
    if (!wsState(workspace).attached) {
        throw new GnoblinError("invalid_state", target, "add the workspace to gnoblin.workspaces first");
    }
    if (!isClient(client) || !ctx.clientsByNode.has(stateOf(client).node)) {
        throw new GnoblinError("not_found", target, "the client is gone or is not a client of the compositor");
    }
    moveClient(client, workspace, true);
}

/** `workspace.clients.remove(client)`: the client has no workspace until it is given one. */
export function detachClient(workspace: Workspace, client: Client): void {
    if (ctx.membership.get(client) === workspace) {
        moveClient(client, null, false);
    }
}

/** Removes every client from a workspace without closing the windows. */
export function clearClients(workspace: Workspace): void {
    for (const client of [...wsState(workspace).clientsState.list] as Client[]) {
        detachClient(workspace, client);
    }
}
