// The frame's tree events: what the shell reports (created, touched and destroyed nodes, pointer edges, the end of
// boot) becomes object changes and queued listener calls.

import { adopt, leave, outputChanged, releaseScreen } from "gnoblin/host/adoption";
import { host } from "gnoblin/host/bridge";
import { CLIENT_KEYS } from "gnoblin/host/client";
import type { Client } from "gnoblin/host/client";
import { clientHolders, emitMember, holders, recompute } from "gnoblin/host/collection_events";
import { ctx } from "gnoblin/host/context";
import type { WindowPointerEvent } from "gnoblin/host/contract";
import { isShown } from "gnoblin/host/desks";
import { queueFor } from "gnoblin/host/dispatch";
import { dialogsOf, forgetArrival, placeDialog } from "gnoblin/host/membership";
import { publishedValues, stateOf } from "gnoblin/host/node_state";
import type { Seen } from "gnoblin/host/node_state";
import { settleApplications } from "gnoblin/host/output_apply";
import { focusedClient } from "gnoblin/host/seat";
import { declareDecorations } from "gnoblin/host/services";
import type { Workspace } from "gnoblin/host/workspace";
import { wsOf, wsState } from "gnoblin/host/workspace_state";
import { adoptOrphans, ensureDefaultWorkspace } from "gnoblin/host/workspaces";
import { noteHostTouched, raiseSignal } from "gnoblin/properties";
import { sceneForget, sceneNodePointer } from "gnoblin/scene";
import type { NodeId } from "gnoblin/types";
import { uiNodeGone } from "gnoblin/ui";
import { windowState } from "gnoblin/window_state";

/**
 * Hands the keyboard to the neighbour of a focused window that closed, as the retired strip did for its columns and every
 * layout now does: the window after it in the workspace, else the one before; a dialog hands it back to its parent. Nothing
 * moves while the workspace is not shown, a minimised window is skipped, and a window the compositor has already focused
 * is left alone. A layout that has its own `onClientRemoved` (a strip's columns are not in window order) does it itself.
 */
function handOffFocus(workspace: Workspace, index: number, parent: Client | null): void {
    if (!isShown(workspace) || host.input.seats().length === 0 || focusedClient() !== null) {
        return;
    }
    const list = wsState(workspace).clientsState.list as Client[];
    const usable = (client: Client | undefined): client is Client =>
        client !== undefined && client.alive && !client.minimised;
    const next = [parent ?? undefined, list[index], list[index - 1]].find(usable);
    next?.focus();
}

/** Releases a destroyed host node and notifies the listeners that held its wrapper. */
function release(node: NodeId): void {
    const gone = ctx.outputsByNode.get(node);
    if (gone !== undefined) {
        releaseScreen(gone);
        return;
    }
    const client = ctx.clientsByNode.get(node);
    if (!client) {
        return;
    }
    const own = wsOf(client);
    const ownHeld = own ? holders(own.clientsState, client) : [];
    const wasFocused = ctx.flags.lastFocus === client;
    const parent = client.parentWindow;
    const held = leave(ctx.clientRoot, client, ctx.clientsByNode, CLIENT_KEYS);
    ctx.input.clientGone(client);
    if (own) {
        const workspace = ctx.membership.get(client) as Workspace;
        ctx.membership.delete(client);
        const index = own.clientsState.list.indexOf(client);
        own.clientsState.list.splice(index, 1);
        raiseSignal(own.clientsState, "items");
        own.dirty = true;
        if (wasFocused && typeof own.layout.onClientRemoved !== "function") {
            handOffFocus(workspace, index, parent);
        }
        forgetArrival(own, client);
        for (const state of ownHeld) {
            state.members?.delete(client);
        }
    }
    const all = [...held, ...ownHeld];
    emitMember(client, all, "onClosed", [client]);
    for (const state of all) {
        queueFor(state, "onRemoved", [client, state.self]);
    }
}

/** Raises the events one client's changed properties imply, to the listeners that held it before the change. */
export function changed(client: Client): void {
    const state = stateOf(client);
    const seen = state.seen as Seen;
    const now = publishedValues(state.node);
    const held = clientHolders(client);
    if (now.title !== seen.title) {
        seen.title = now.title;
        emitMember(client, held, "onTitleChanged", [client, now.title]);
    }
    if (now.appId !== seen.appId) {
        seen.appId = now.appId;
        emitMember(client, held, "onAppIdChanged", [client, now.appId]);
    }
    if (now.decoration !== seen.decoration) {
        seen.decoration = now.decoration;
        emitMember(client, held, "onDecorationChanged", [client, now.decoration]);
        // The bar's room above the window came or went: the workspace's layout places it again.
        const own = wsOf(client);
        if (own) {
            own.dirty = true;
        }
    }
    if (now.focused !== seen.focused) {
        seen.focused = now.focused;
        emitMember(client, held, "onFocusChanged", [client, now.focused]);
    }
    if (now.x !== seen.x || now.y !== seen.y || now.width !== seen.width || now.height !== seen.height) {
        const { x, y, width, height } = now;
        const [dx, dy] = [x - seen.x, y - seen.y];
        Object.assign(seen, { x, y, width, height });
        emitMember(client, held, "onGeometryChanged", [client, { x, y, width, height }]);
        // A dialog moves with its parent.
        if (dx !== 0 || dy !== 0) {
            for (const dialog of dialogsOf(client)) {
                dialog.x += dx;
                dialog.y += dy;
            }
        }
    }
    if (now.parent !== seen.parent) {
        seen.parent = now.parent;
        if (ctx.flags.opened && now.parent !== 0) {
            placeDialog(client);
        }
    }
}

/** Processes one batch of host tree changes and returns the queued callback count. */
export function plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number {
    ctx.launches.frame();
    settleApplications();
    // The destroyed list is deepest first. Departures go before arrivals so a collection never holds a
    // dead member while it is told of a new one.
    for (const node of destroyed) {
        uiNodeGone(node);
        release(node);
    }
    sceneForget(destroyed);
    for (const node of created) {
        adopt(node, true);
    }
    noteHostTouched(touched);
    for (const node of new Set(touched)) {
        const output = ctx.outputsByNode.get(node);
        if (output) {
            outputChanged(output);
            recompute(ctx.outputRoot, output);
        }
        const client = ctx.clientsByNode.get(node);
        if (client) {
            changed(client);
            recompute(ctx.clientRoot, client);
            const own = wsOf(client);
            if (own) {
                recompute(own.clientsState, client);
            }
        }
    }
    return ctx.queue.length;
}

/** The listener method that hears each pointer edge on a window. */
const POINTER_METHODS: Readonly<Record<string, string>> = {
    enter: "onPointerEnter",
    motion: "onPointerMotion",
    leave: "onPointerLeave",
    press: "onPointerPress",
    release: "onPointerRelease",
};

/** Queues the clients' pointer listeners for the edges the compositor reported on windows this frame. */
export function pointer(events: WindowPointerEvent[]): number {
    for (const event of events) {
        const client = ctx.clientsByNode.get(event.node);
        if (client === undefined) {
            // Not a window: a View or Node a script made, whose pointer edges the shell is sent (a View in `capture`).
            const { x, y, button } = event;
            const task = sceneNodePointer(event.node, event.kind, { x, y, button });
            if (task !== undefined) {
                ctx.queue.push({ site: `node ${event.node}.${event.kind}`, task });
            }
            continue;
        }
        windowState.pointerEdge(event.kind);
        const method = POINTER_METHODS[event.kind];
        if (client && method && stateOf(client).alive) {
            const { x, y, button } = event;
            emitMember(client, clientHolders(client), method, [client, { x, y, button }]);
        }
    }
    return ctx.queue.length;
}

/** Queues `onReady` for the clients that were there at boot, parents first, once the config has loaded. */
export function ready(nodes: NodeId[]): number {
    ensureDefaultWorkspace();
    ctx.flags.opened = true;
    adoptOrphans();
    // The declaration is written here, after the generation that ended took its own back: a reload stages the new
    // generation before it retires the old one, so a write made while staging would be undone by that retire.
    if (ctx.decorations !== null) {
        declareDecorations(true);
    }
    for (const node of nodes) {
        const client = ctx.clientsByNode.get(node);
        if (client && !stateOf(client).readied) {
            stateOf(client).readied = true;
            emitMember(client, clientHolders(client), "onReady", [client]);
        }
    }
    return ctx.queue.length;
}
