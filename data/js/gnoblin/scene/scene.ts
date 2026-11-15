// `Scene`: the one object `gnoblin.scene` is, and the root of the tree the compositor keeps.

import { host, SceneError, sceneState } from "gnoblin/scene/backend";
import { Node } from "gnoblin/scene/node";
import { label } from "gnoblin/scene/registry";

/** The root of the tree the compositor keeps: the node a script mounts its own nodes under. It cannot be destroyed. */
class SceneRoot extends Node {
    override destroy(): void {
        throw new SceneError("denied", label(this), "the scene's root is the compositor's and cannot be destroyed");
    }
}

const SCENE = Symbol("gnoblin.scene");

let sceneRoot: SceneRoot | null = null;

let theScene: Scene | null = null;

let sceneMetres = 1;

/**
 * The scene the shell draws: its `root` holds the nodes a script makes (`Node`, `View`, `Camera3D`). There is one,
 * `gnoblin.scene`. Clients and workspaces are the compositor's and a workspace is not a tree node, so they are
 * reached through `gnoblin.clients` and `gnoblin.workspaces`, and `root.children` lists script-made nodes only.
 */
export class Scene {
    constructor(token: symbol) {
        if (token !== SCENE) {
            throw new TypeError("the scene is `gnoblin.scene`; a script does not make one");
        }
    }

    /** The stable id of the one scene. */
    get id(): string {
        return "scene";
    }

    /** Always true: the scene lasts as long as the shell. */
    get alive(): boolean {
        return true;
    }

    /** The root node of the tree. `scene.root.add(room)` mounts a room under it. */
    get root(): Node {
        if (sceneRoot === null) {
            sceneState.wrapping = host().root();
            try {
                sceneRoot = new SceneRoot();
            } finally {
                sceneState.wrapping = null;
            }
        }
        return sceneRoot;
    }

    /**
     * Metres in one scene unit: 1 until set, which is the specification's default of logical units. The script keeps
     * it; a scene that is read by the XR program carries its unit on its camera (`Camera3D.metres`).
     */
    get metresPerUnit(): number {
        return sceneMetres;
    }
    set metresPerUnit(value: number) {
        if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
            throw new TypeError("Scene.metresPerUnit: must be a finite number above 0");
        }
        sceneMetres = value;
    }
}

/** The one scene, made on first use. `gnoblin.scene` returns it. */
export function sceneOf(): Scene {
    theScene ??= new Scene(SCENE);
    return theScene;
}
