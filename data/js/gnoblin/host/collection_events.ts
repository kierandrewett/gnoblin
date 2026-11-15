// Tracks collection membership and queues events when members join, leave or change.

import type { Client } from "gnoblin/host/client";
import { contains, rootOf } from "gnoblin/host/collection_state";
import type { CollectionState, Member } from "gnoblin/host/collection_state";
import { ctx } from "gnoblin/host/context";
import { queueFor } from "gnoblin/host/dispatch";
import { stateOf } from "gnoblin/host/node_state";
import type { Output } from "gnoblin/host/output";
import { wsOf } from "gnoblin/host/workspace_state";

/** The collections that hold `member` now: its root and every tracked view with it as a member. */
export function holders(top: CollectionState, member: Member): CollectionState[] {
    return [top, ...ctx.tracked.filter((view) => view.members?.has(member) && rootOf(view) === top)];
}

/** Tells every tracked view of `top` that `member` joined or left, when its query says so. */
export function recompute(top: CollectionState, member: Member): void {
    for (const view of ctx.tracked) {
        if (rootOf(view) !== top || view.members === null) {
            continue;
        }
        const was = view.members.has(member);
        const is = contains(view, member);
        if (was && !is) {
            view.members.delete(member);
            queueFor(view, "onRemoved", [member, view.self]);
        } else if (!was && is) {
            view.members.add(member);
            queueFor(view, "onAdded", [member, view.self]);
        }
    }
}

/** Queues a member event for its own listeners and every collection that currently holds it. */
export function emitMember(client: Client | Output, held: CollectionState[], method: string, args: unknown[]): void {
    queueFor(stateOf(client), method, args);
    for (const state of held) {
        queueFor(state, method, args);
    }
}

/** Every collection that holds `client` now: `gnoblin.clients`, its workspace's `clients`, and its live
 * selections.
 */
export function clientHolders(client: Client): CollectionState[] {
    const own = wsOf(client);
    const all = holders(ctx.clientRoot, client);
    return own ? [...all, ...holders(own.clientsState, client)] : all;
}
