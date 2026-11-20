import type { NodeId } from "gnoblin-std/scene/backend";
import type { Node } from "gnoblin-std/scene/node";
/** Adds a node to the scene root if it has no parent. */
export declare function mountOnRoot(node: Node): void;
/**
 * Adds `node` under a host-managed parent, such as a UI canvas. A node already in the tree moves to the new parent.
 */
export declare function mountUnder(node: Node, parent: NodeId): void;
/** Maps each scene object to its host node id. */
export declare const nodes: WeakMap<object, number>;
/** Maps live host node ids to their scene objects. */
export declare const live: Map<number, Node>;
/** Tracks scene objects whose host nodes have been destroyed. */
export declare const gone: WeakSet<object>;
/** Stores the stable ids supplied when script-created nodes are constructed. */
export declare const givenIds: WeakMap<object, string>;
/** Stores metres per unit for nodes whose host tree has no such property. */
export declare const metresPerUnit: WeakMap<object, number>;
/** Returns the host node id for a scene object, or throws for an unknown object. */
export declare function nodeIdOf(object: Node): NodeId;
/** Returns the scene object for a host node id, if this module created it. */
export declare function sceneNode(id: NodeId): Node | undefined;
/**
 * Returns a deferred call to a node's pointer handler, or undefined if the node or handler is unavailable. The shell
 * queues the call with listener calls so handler errors are reported rather than thrown immediately.
 */
export declare function sceneNodePointer(id: NodeId, kind: string, point: {
    readonly x: number;
    readonly y: number;
    readonly button: number;
}): (() => void) | undefined;
/** Records nodes destroyed outside this module and disposes their property bindings. */
export declare function sceneForget(ids: readonly NodeId[]): void;
/**
 * Destroys nodes created in the current generation that have no scene parent of their own, including their subtrees.
 * The host calls this when the generation ends.
 */
export declare function sceneReleaseAll(): void;
/** Formats a node's class and host id for error messages. */
export declare function label(object: Node): string;
