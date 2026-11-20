import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import { Output } from "gnoblin-core/host/output";
import type { NodeId } from "gnoblin-core/types";
/**
 * Wraps a node the first time it appears. Nodes added after startup are announced to collections and listeners.
 * Nodes already present at startup are not announced because the config attaches after they exist.
 */
export declare function adopt(node: NodeId, announce: boolean): void;
/** Wraps the nodes the compositor already had when the host booted: its screens first, then its windows. */
export declare function adoptExisting(): void;
/** Releases a destroyed screen: it leaves `gnoblin.outputs` and its listeners hear that it is gone. */
export declare function releaseScreen(output: Output): void;
/** Queues events for changes to an output's published properties. */
export declare function outputChanged(output: Output): void;
/** Removes a destroyed member from its collections after saving its final readable values. */
export declare function leave(top: CollectionState, member: Member, map: Map<NodeId, Member>, keys: string[]): CollectionState[];
