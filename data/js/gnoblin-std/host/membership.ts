// Which windows are on which workspace, and the moves of a window between them.
//
// The library keeps the membership (`libgnoblin-std`'s workspaces and their collections of clients). What a script holds are
// `MutableCollection`s of the engine's `Client` objects, one for each workspace and one for `gnoblin.workspaces`, with the
// listeners and the live selections a script puts on them. This module keeps those lists equal to the library's: after each
// call that can change a membership, when the library announces one, and at the start of every frame it reads the library's
// list and does for each difference what the host always did for a window joining or leaving (the signal, the listeners, the
// selections). A window the engine has not seen yet is left for the next time.

import { GnoblinError } from "gnoblin-core/errors";
import type { Client } from "gnoblin-core/host/client";
import { holders, recompute } from "gnoblin-core/host/collection_events";
import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import { ctx } from "gnoblin-core/host/context";
import { queueFor } from "gnoblin-core/host/dispatch";
import { isClient, stateOf } from "gnoblin-core/host/node_state";
import { disposeBindingsOf, raiseSignal } from "gnoblin-core/properties";
import { isKnown, whenKnown } from "gnoblin-std/host/client_state";
import { nativeClient } from "gnoblin-std/host/library";
import { std } from "gnoblin-std/host/state";
import type { Workspace } from "gnoblin-std/host/workspace";
import { isWorkspace, workspaceOf, wsState } from "gnoblin-std/host/workspace_state";
import { Collection as NativeCollection, Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import { CollectionKind } from "gnoblin-std/generated/constants";

/** Whether two lists hold the same objects in the same order. */
function sameList(a: readonly unknown[], b: readonly unknown[]): boolean {
    return a.length === b.length && a.every((member, at) => member === b[at]);
}

/** Puts `list` right where it is, so whoever holds the array sees the change. */
function replaceIn(list: Member[], next: readonly Member[]): void {
    list.splice(0, list.length, ...next);
}

/** Makes `gnoblin.workspaces` the list the library holds, and tells the listeners and the selections what it changed. */
function syncWorkspaceList(): void {
    const root = std.workspaceRoot;
    const handles = NativeCollection.root(CollectionKind.WORKSPACES)?.members() ?? [];
    const next = handles.map((handle) => workspaceOf(new NativeWorkspace(handle)));
    const list = root.list as Workspace[];
    if (sameList(list, next)) {
        return;
    }
    const left = list.filter((workspace) => !next.includes(workspace));
    const joined = next.filter((workspace) => !list.includes(workspace));
    const held = new Map(left.map((workspace) => [workspace, holders(root, workspace)] as const));
    replaceIn(root.list, next);
    std.displayManager.changed();
    for (const workspace of left) {
        wsState(workspace).attached = false;
        disposeBindingsOf(workspace);
        for (const view of held.get(workspace) ?? []) {
            view.members?.delete(workspace);
            queueFor(view, "onRemoved", [workspace, view.self]);
        }
    }
    for (const workspace of joined) {
        wsState(workspace).attached = true;
        queueFor(root, "onAdded", [workspace, root.self]);
    }
    raiseSignal(root, "items");
    // A workspace's index moved with the list, and which of them is active may have with it.
    for (const workspace of next) {
        recompute(root, workspace);
    }
}

/** Puts `client` in the list of `workspace`, which it is not in. */
function link(state: CollectionState, workspace: Workspace, client: Client): void {
    std.membership.set(client, workspace);
    state.list.push(client);
    raiseSignal(state, "items");
    queueFor(state, "onAdded", [client, state.self]);
    recompute(state, client);
}

/** Takes `client` out of the list of `workspace`. */
function unlink(state: CollectionState, workspace: Workspace, client: Client): void {
    if (std.membership.get(client) === workspace) {
        std.membership.delete(client);
    }
    state.list.splice(state.list.indexOf(client), 1);
    raiseSignal(state, "items");
    queueFor(state, "onRemoved", [client, state.self]);
    recompute(state, client);
}

/**
 * Makes every list of windows the library's. A window leaves its old workspace's list before it joins the new one's, as a
 * move always told them. A window the engine has not seen is not in the list yet, and the next call finds it.
 */
export function syncMembership(): void {
    syncWorkspaceList();
    const wanted = new Map<Workspace, Client[]>();
    for (const workspace of std.byHandle.values()) {
        const nodes = wsState(workspace).nativeClients.members();
        wanted.set(
            workspace,
            nodes.flatMap((node) => ctx.clientsByNode.get(node) ?? []),
        );
    }
    for (const [workspace, next] of wanted) {
        const state = wsState(workspace).clientsState;
        for (const client of [...state.list] as Client[]) {
            if (!next.includes(client)) {
                unlink(state, workspace, client);
            }
        }
    }
    for (const [workspace, next] of wanted) {
        const state = wsState(workspace).clientsState;
        for (const client of next) {
            if (!state.list.includes(client)) {
                link(state, workspace, client);
            }
        }
        // The same windows in another order: the workspace's windows traded places.
        if (!sameList(state.list, next)) {
            replaceIn(state.list, next);
            raiseSignal(state, "items");
        }
    }
}

/** A window closed: it leaves its workspace's list at once, as the compositor's word of it reaches the engine first. */
export function forgetClient(client: Client): void {
    const workspace = std.membership.get(client);
    if (workspace === undefined) {
        return;
    }
    const state = wsState(workspace).clientsState;
    std.membership.delete(client);
    const at = state.list.indexOf(client);
    if (at >= 0) {
        state.list.splice(at, 1);
        raiseSignal(state, "items");
    }
    for (const view of holders(state, client)) {
        view.members?.delete(client);
    }
}

/** The moves asked of windows the library does not know yet, made when it does. */
const waiting = new Map<Client, Workspace>();

/**
 * Asks the library to move `client` to `workspace`. The engine hears of a window a turn before the library, so a move asked of a
 * window the library does not know yet waits for the next frame: it is taken as done when nothing that the engine can see
 * would refuse it.
 */
function moveWhenKnown(client: Client, workspace: Workspace, ask: () => boolean): boolean {
    const moved = whenKnown(ask, () => {
        const refused =
            !client.alive ||
            client.fullscreen ||
            client.parentWindow !== null ||
            !wsState(workspace).attached ||
            std.membership.get(client) === workspace;
        if (!refused) {
            waiting.set(client, workspace);
        }
        return !refused;
    });
    syncMembership();
    return moved;
}

/** Makes the moves that waited for the library to know their window. */
export function flushMoves(): void {
    for (const [client, workspace] of [...waiting]) {
        if (!client.alive) {
            waiting.delete(client);
        } else if (isKnown(client)) {
            waiting.delete(client);
            nativeClient(client).moveToWorkspace(wsState(workspace).native);
        }
    }
}

/** `client.moveToWorkspace(workspace)`: the retired shell's `Window:move_to`. */
export function moveClientTo(client: Client, workspace: Workspace): boolean {
    if (!isWorkspace(workspace)) {
        throw new TypeError(`${client}.moveToWorkspace: needs a Workspace`);
    }
    return moveWhenKnown(client, workspace, () => nativeClient(client).moveToWorkspace(wsState(workspace).native));
}

/** `workspace.clients.add(client)`: the client leaves its workspace and joins this one. */
export function transferClient(workspace: Workspace, client: Client): void {
    const target = `${workspace}.clients.add`;
    const state = wsState(workspace);
    if (state.native.info().attached === 0) {
        throw new GnoblinError("invalid_state", target, "add the workspace to gnoblin.workspaces first");
    }
    if (!isClient(client) || !ctx.clientsByNode.has(stateOf(client).node)) {
        throw new GnoblinError("not_found", target, "the client is gone or is not a client of the compositor");
    }
    moveWhenKnown(client, workspace, () => {
        state.nativeClients.add(stateOf(client).node);
        return true;
    });
}

/** `workspace.clients.remove(client)`: the client has no workspace until it is given one. */
export function detachClient(workspace: Workspace, client: Client): void {
    wsState(workspace).nativeClients.remove(stateOf(client).node);
    syncMembership();
}

/** Removes every client from a workspace without closing the windows. */
export function clearClients(workspace: Workspace): void {
    wsState(workspace).nativeClients.clear();
    syncMembership();
}

/** A workspace's clients traded places: its layout computes again, and one with a record of its own trades them there. */
export function swapClients(workspace: Workspace, a: Client, b: Client): void {
    wsState(workspace).nativeClients.swap(stateOf(a).node, stateOf(b).node);
    syncMembership();
}
