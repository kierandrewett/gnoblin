// Tracks scene objects by tree node id and handles pointer events and nodes destroyed by the compositor.

import { errorCodeOfRefusal } from "gnoblin-core/errors";
import { disposeBindingsOf } from "gnoblin-core/properties";
import { host, SceneError } from "gnoblin-std/scene/backend";
import type { NodeId } from "gnoblin-std/scene/backend";
import type { Node } from "gnoblin-std/scene/node";

/** Adds a node to the scene root if it has no parent. */
export function mountOnRoot(node: Node): void {
    if (host().parent(nodeIdOf(node)) !== undefined) {
        return;
    }
    const code = host().insert(nodeIdOf(node), host().root(), -1);
    if (code !== undefined) {
        throw new SceneError(
            errorCodeOfRefusal(code),
            `${label(node)}.mount`,
            `the shell refused to mount the node (${code})`,
        );
    }
}

/**
 * Adds `node` under a host-managed parent, such as a UI canvas. A node already in the tree moves to the new parent.
 */
export function mountUnder(node: Node, parent: NodeId): void {
    const code = host().insert(nodeIdOf(node), parent, -1);
    if (code !== undefined) {
        throw new SceneError(
            errorCodeOfRefusal(code),
            `${label(node)}.mount`,
            `the shell refused to mount the node (${code})`,
        );
    }
}

/** Maps each scene object to its host node id. */
export const nodes = new WeakMap<object, NodeId>();

/** Maps live host node ids to their scene objects. */
export const live = new Map<NodeId, Node>();

/** Tracks scene objects whose host nodes have been destroyed. */
export const gone = new WeakSet<object>();

/** Stores the stable ids supplied when script-created nodes are constructed. */
export const givenIds = new WeakMap<object, string>();

/** Stores metres per unit for nodes whose host tree has no such property. */
export const metresPerUnit = new WeakMap<object, number>();

/** Returns the host node id for a scene object, or throws for an unknown object. */
export function nodeIdOf(object: Node): NodeId {
    const id = nodes.get(object);
    if (id === undefined) {
        throw new TypeError("not a scene node");
    }
    return id;
}

/** Returns the scene object for a host node id, if this module created it. */
export function sceneNode(id: NodeId): Node | undefined {
    return live.get(id);
}

/** Node member called for each supported pointer edge. */
type PointerHandler = "onPointerEnter" | "onPointerMotion" | "onPointerLeave" | "onPointerPress" | "onPointerRelease";

const POINTER_HANDLERS: Readonly<Record<string, PointerHandler>> = {
    enter: "onPointerEnter",
    motion: "onPointerMotion",
    leave: "onPointerLeave",
    press: "onPointerPress",
    release: "onPointerRelease",
};

/**
 * Returns a deferred call to a node's pointer handler, or undefined if the node or handler is unavailable. The shell
 * queues the call with listener calls so handler errors are reported rather than thrown immediately.
 */
export function sceneNodePointer(
    id: NodeId,
    kind: string,
    point: { readonly x: number; readonly y: number; readonly button: number },
): (() => void) | undefined {
    const node = live.get(id);
    const member = POINTER_HANDLERS[kind];
    const handler = node !== undefined && member !== undefined ? node[member] : undefined;
    if (node === undefined || typeof handler !== "function") {
        return undefined;
    }
    return () => handler.call(node, { node, x: point.x, y: point.y, button: point.button });
}

/** Records nodes destroyed outside this module and disposes their property bindings. */
export function sceneForget(ids: readonly NodeId[]): void {
    for (const id of ids) {
        const object = live.get(id);
        if (object !== undefined) {
            live.delete(id);
            gone.add(object);
            disposeBindingsOf(object);
        }
    }
}

/**
 * Destroys nodes created in the current generation that have no scene parent of their own, including their subtrees.
 * The host calls this when the generation ends.
 */
export function sceneReleaseAll(): void {
    for (const [id, object] of [...live]) {
        if (!live.has(id) || object.parent !== null) {
            continue;
        }
        try {
            object.destroy();
        } catch {
            // The node is gone already, or the shell refused: nothing is left to release.
        }
    }
}

/** Formats a node's class and host id for error messages. */
export function label(object: Node): string {
    return `${object.constructor.name}#${nodes.get(object) ?? "?"}`;
}
