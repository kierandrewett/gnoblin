import type { Client } from "gnoblin-core/host/client";
import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import type { Output } from "gnoblin-core/host/output";
/** The collections that hold `member` now: its root and every tracked view with it as a member. */
export declare function holders(top: CollectionState, member: Member): CollectionState[];
/** Tells every tracked view of `top` that `member` joined or left, when its query says so. */
export declare function recompute(top: CollectionState, member: Member): void;
/** Queues a member event for its own listeners and every collection that currently holds it. */
export declare function emitMember(client: Client | Output, held: CollectionState[], method: string, args: unknown[]): void;
/**
 * Every collection that holds `client` now: `gnoblin.clients`, the other roots a package keeps it in (a workspace's
 * `clients`), and their live selections.
 */
export declare function clientHolders(client: Client): CollectionState[];
