// What the scene classes need of the host: the `SceneBackend` the shell's Rust host implements, the error a failed
// operation throws, and the one table that holds the backend once `attachScene` has given it. The scene objects do not
// reach the host any other way, so they run under a fake in tests.

import { GnoblinError } from "gnoblin/errors";
import type { ErrorCode } from "gnoblin/errors";
import type { WorkspaceSource } from "gnoblin/scene_mirror";

/** What the scene classes share with the host: its backend, and the tree node a constructor must wrap instead of make. */
export const sceneState: {
    /** The host the module runs on, given once by `attachScene`. */
    backend: SceneBackend | null;
    /** Set while the scene root is being wrapped: the node already exists in the tree, so the constructor does not make one. */
    wrapping: NodeId | null;
} = { backend: null, wrapping: null };

/** A node of the compositor tree, by its number. */
export type NodeId = number;

/** A value a node property holds that is one thing. */
export type Scalar = string | number | boolean;

/** What the host reads and writes for a node property. */
export type HostValue = Scalar | number[] | string[];

/** What the scene classes need of the host. The shell's Rust host implements it. */
export interface SceneBackend {
    /** Makes a node of `class` (`Node`, `Camera` or `View`): its id, or the refusal code as text. */
    create(cls: string): NodeId | string;
    /** Puts `node` under `parent` at `index` (negative is last); a node already in the tree moves. */
    insert(node: NodeId, parent: NodeId, index: number): string | undefined;
    destroy(node: NodeId): string | undefined;
    children(node: NodeId): NodeId[];
    parent(node: NodeId): NodeId | undefined;
    /** The root of the tree: where a room is mounted. */
    root(): NodeId;
    get(node: NodeId, key: string): HostValue | undefined;
    set(node: NodeId, key: string, value: HostValue): string | undefined;
    /** The node a client is, or undefined for an object that is not one. */
    nodeOf(content: object): NodeId | undefined;
    /** The client a node is, or undefined. */
    contentOf(node: NodeId): object | undefined;
    /** The workspace `content` is, as what a View of it reads, or undefined for an object that is not one. */
    workspace(content: object): WorkspaceSource | undefined;
}

/** What a failed operation on a scene node throws: a `GnoblinError` that a script can also catch by its own type. */
export class SceneError extends GnoblinError {
    constructor(code: ErrorCode, target: string, message: string) {
        super(code, target, message);
        this.name = "SceneError";
    }
}

/** Gives the module its backend. The host module calls it once, before any user code runs. */
export function attachScene(next: SceneBackend): void {
    sceneState.backend = next;
}

/** The backend, or a thrown error when the module has none yet. */
export function host(): SceneBackend {
    if (sceneState.backend === null) {
        throw new Error("the scene needs the shell host and cannot run without it");
    }
    return sceneState.backend;
}
