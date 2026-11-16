// Tracks collection membership and queues events when members join, leave or change.

import type { Client } from "gnoblin-core/host/client";
import { contains, rootOf } from "gnoblin-core/host/collection_state";
import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import { ctx } from "gnoblin-core/host/context";
import { queueFor } from "gnoblin-core/host/dispatch";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { hooks } from "gnoblin-core/host/hooks";

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

/**
 * Every collection that holds `client` now: `gnoblin.clients`, the other roots a package keeps it in (a workspace's
 * `clients`), and their live selections.
 */
export function clientHolders(client: Client): CollectionState[] {
    return [ctx.clientRoot, ...hooks.clientRoots(client)].flatMap((root) => holders(root, client));
}
