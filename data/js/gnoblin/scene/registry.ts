// Which script object is which tree node: the object of each node id, the ids scripts gave their nodes, the nodes the
// compositor destroyed under us, and the pointer edges a node's handler hears. A node reached by id is the same
// object as the one that made it.

import { errorCodeOfRefusal } from "gnoblin/errors";
import { disposeBindingsOf } from "gnoblin/properties";
import { host, SceneError } from "gnoblin/scene/backend";
import type { NodeId } from "gnoblin/scene/backend";
import type { Node } from "gnoblin/scene/node";

/** Puts `node` last under the tree's root, which is where the XR program looks for the room. A node in the tree stays. */
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
 * Puts `node` last under `parent`, a node of the tree that is not a scene node of this module (a canvas of the UI
 * toolkit, say). A node already in the tree moves. This is how a `View` goes into an outlet of a window frame.
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

/** The node a script object stands for, for the objects of this module. */
export const nodes = new WeakMap<object, NodeId>();

/** Every live scene object by node id, so a tree node reached by id is the same object as the one that made it. */
export const live = new Map<NodeId, Node>();

/** The scene objects whose tree node is gone, so a stale handle answers `alive` false. */
export const gone = new WeakSet<object>();

/** The ids scripts gave their nodes (`new Node({ id })`): `node.id` answers with them. */
export const givenIds = new WeakMap<object, string>();

/** Metres in one unit of a node's space, kept by the script because ordinary nodes have no tree property for it. */
export const metresPerUnit = new WeakMap<object, number>();

/** The tree node of a scene object. */
export function nodeIdOf(object: Node): NodeId {
    const id = nodes.get(object);
    if (id === undefined) {
        throw new TypeError("not a scene node");
    }
    return id;
}

/** The scene object for node `id`, or undefined when this module did not make it. */
export function sceneNode(id: NodeId): Node | undefined {
    return live.get(id);
}

/** The handler member of a node that hears each pointer edge. */
type PointerHandler = "onPointerEnter" | "onPointerMotion" | "onPointerLeave" | "onPointerPress" | "onPointerRelease";

const POINTER_HANDLERS: Readonly<Record<string, PointerHandler>> = {
    enter: "onPointerEnter",
    motion: "onPointerMotion",
    leave: "onPointerLeave",
    press: "onPointerPress",
    release: "onPointerRelease",
};

/**
 * The call that tells a node's handler of a pointer edge, or undefined when this module did not make the node or it
 * has no handler for the edge. The shell queues it beside the listeners' calls so a throw is reported, not raised.
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

/** Tells the module nodes were destroyed by someone else (the compositor, a destroyed ancestor). */
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
 * Destroys every node this generation made that has no scene parent of its own, with their subtrees. The host calls
 * it when the generation ends, so a reload does not leave the old generation's room and Views behind.
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

/** How an error names a node: its class and tree id. */
export function label(object: Node): string {
    return `${object.constructor.name}#${nodes.get(object) ?? "?"}`;
}
