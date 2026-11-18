// The scene backend over the library (`gnoblin-std/generated`'s `Scene`). A node a script makes is the library's: it makes it, checks what is
// written to it (finite numbers, the projection names, a zoom above zero), keeps what the tree has no property for (a camera's `size`) and
// follows a transition's staging. A node the compositor made (a window, the XR program's head and controllers) is not the library's, so
// reads and writes of those go through the engine's own mirror, as they always did.
//
// The library reads the tree as the shell's own writes leave it, so a node made a moment ago is there when it is written, inserted or
// asked for its children; the engine's mirror learns it when the compositor echoes it, which is why a script's node is never read there.

import { noteHostRead, noteHostWrite } from "gnoblin-core/properties";
import { PropKind, SceneKind } from "gnoblin-std/generated/constants";
import { Scene } from "gnoblin-std/generated/classes";
import { Prop } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import { refusalOf, stdFailure, wrongKind } from "gnoblin-std/native_error";
import type { HostValue, NodeId, SceneBackend } from "gnoblin-std/scene/backend";

/** The library's kind of each class a script may make. */
const KINDS: Readonly<Record<string, number>> = {
    Node: SceneKind.NODE,
    Camera: SceneKind.CAMERA3D,
    View: SceneKind.VIEW,
    Camera2D: SceneKind.CAMERA2D,
};

/** The properties that name another node: a number written to one is a node. */
const NODE_KEYS = ["source", "camera"];

/** A value the way the library takes it, or nothing for one it does not (a list of texts). */
function propOf(key: string, value: HostValue): Init<Prop> | undefined {
    if (typeof value === "number") {
        return NODE_KEYS.includes(key) ? Prop.node(value) : Prop.number(value);
    }
    if (typeof value === "boolean") {
        return Prop.bool(value ? 1 : 0);
    }
    if (typeof value === "string") {
        return Prop.text(value);
    }
    if (Array.isArray(value) && value.length === 3 && value.every((part) => typeof part === "number")) {
        return Prop.triple(value[0] as number, value[1] as number, value[2] as number);
    }
    return undefined;
}

/** What the library answered, as a script reads it. */
function valueOf(prop: Prop): HostValue | undefined {
    switch (prop.kind) {
        case PropKind.NUMBER:
            return prop.a;
        case PropKind.BOOL:
            return prop.a !== 0;
        case PropKind.TRIPLE:
            return [prop.a, prop.b, prop.c];
        case PropKind.TEXT:
            return prop.text;
        case PropKind.NODE:
            return prop.node;
        default:
            return undefined;
    }
}

/**
 * The scene's backend: what a script makes is the library's, everything else is the engine's. `engine` answers for the nodes the
 * library did not make and for what only the script's objects know (the clients and workspaces a View shows).
 */
export function nativeScene(engine: SceneBackend): SceneBackend {
    const made = new Set<NodeId>();
    let root: NodeId | null = null;
    const rootId = (): NodeId => {
        root ??= Scene.root();
        if (root === null) {
            throw new Error("the library has no scene root");
        }
        return root;
    };
    const ours = (node: NodeId): boolean => made.has(node) || node === rootId();
    return {
        create(cls) {
            const kind = KINDS[cls];
            if (kind === undefined) {
                return "unknown-class";
            }
            try {
                const id = Scene.new(kind);
                if (id === null) {
                    return "refused";
                }
                made.add(id);
                return id;
            } catch (error) {
                return refusalOf(error);
            }
        },
        insert(node, parent, index) {
            if (!ours(node)) {
                return engine.insert(node, parent, index);
            }
            try {
                Scene.insert(parent, node, index);
                return undefined;
            } catch (error) {
                return refusalOf(error);
            }
        },
        destroy(node) {
            if (!ours(node)) {
                return engine.destroy(node);
            }
            const gone = [node];
            for (let at = 0; at < gone.length; at += 1) {
                gone.push(...(Scene.children(gone[at]) as number[]));
            }
            try {
                Scene.destroy(node);
            } catch (error) {
                return refusalOf(error);
            }
            for (const id of gone) {
                made.delete(id);
            }
            return undefined;
        },
        children(node) {
            return ours(node) ? (Scene.children(node) as number[]) : engine.children(node);
        },
        parent(node) {
            if (!ours(node)) {
                return engine.parent(node);
            }
            const above = Scene.parent(node);
            return above === null || above === 0 ? undefined : above;
        },
        root: rootId,
        get(node, key) {
            if (!ours(node)) {
                return engine.get(node, key);
            }
            const value = valueOf(Scene.get(node, key));
            noteHostRead(node, key, value);
            return value;
        },
        set(node, key, value) {
            if (!ours(node)) {
                return engine.set(node, key, value);
            }
            const prop = propOf(key, value);
            if (prop === undefined) {
                return "invalid-value";
            }
            try {
                Scene.set(node, key, prop);
            } catch (error) {
                // A value of the wrong kind or out of range is the script's mistake, named by the library with its path.
                if (wrongKind(error)) {
                    throw stdFailure(error, "gnoblin.scene");
                }
                return refusalOf(error);
            }
            noteHostWrite(node, key, value);
            return undefined;
        },
        nodeOf: engine.nodeOf,
        contentOf: engine.contentOf,
        workspace: engine.workspace,
    };
}
