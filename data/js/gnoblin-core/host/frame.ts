// Processes host tree changes, pointer events and readiness into object updates and queued listener calls.

import { adopt, leave, outputChanged, releaseScreen } from "gnoblin-core/host/adoption";
import { CLIENT_KEYS } from "gnoblin-core/host/client";
import type { Client } from "gnoblin-core/host/client";
import { clientHolders, emitMember, holders, recompute } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import type { WindowPointerEvent } from "gnoblin-core/host/contract";
import { queueFor } from "gnoblin-core/host/dispatch";
import { hooks } from "gnoblin-core/host/hooks";
import type { ClientChange } from "gnoblin-core/host/hooks";
import { publishedValues, stateOf } from "gnoblin-core/host/node_state";
import type { Seen } from "gnoblin-core/host/node_state";
import { settleApplications } from "gnoblin-core/host/output_apply";
import { noteHostTouched } from "gnoblin-core/properties";
import type { NodeId } from "gnoblin-core/types";

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
    const roots = hooks.clientRoots(client).flatMap((root) => holders(root, client));
    const parent = client.parentWindow;
    const held = leave(ctx.clientRoot, client, ctx.clientsByNode, CLIENT_KEYS);
    ctx.input.clientGone(client);
    hooks.clientReleased(client, parent);
    const all = [...held, ...roots];
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
    const change: ClientChange = { decoration: false, moved: null, parent: false };
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
        change.decoration = true;
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
        if (dx !== 0 || dy !== 0) {
            change.moved = { dx, dy };
        }
    }
    if (now.parent !== seen.parent) {
        seen.parent = now.parent;
        change.parent = ctx.flags.opened && now.parent !== 0;
    }
    hooks.clientChange(client, change);
}

/** Processes one batch of host tree changes and returns the queued callback count. */
export function plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number {
    ctx.launches.frame();
    settleApplications();
    // The destroyed list is deepest first. Departures go before arrivals so a collection never holds a
    // dead member while it is told of a new one.
    for (const node of destroyed) {
        hooks.nodeDestroyed(node);
        release(node);
    }
    hooks.nodesDestroyed(destroyed);
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
            for (const root of hooks.clientRoots(client)) {
                recompute(root, client);
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
            // Not a window: a node a package made, whose pointer edges the shell is sent (a View in `capture`).
            const { x, y, button } = event;
            const task = hooks.nodePointer(event.node, event.kind, { x, y, button });
            if (task !== undefined) {
                ctx.queue.push({ site: `node ${event.node}.${event.kind}`, task });
            }
            continue;
        }
        hooks.pointerEdge(event.kind);
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
    hooks.finishLoad();
    ctx.flags.opened = true;
    hooks.opened();
    for (const node of nodes) {
        const client = ctx.clientsByNode.get(node);
        if (client && !stateOf(client).readied) {
            stateOf(client).readied = true;
            emitMember(client, clientHolders(client), "onReady", [client]);
        }
    }
    return ctx.queue.length;
}
