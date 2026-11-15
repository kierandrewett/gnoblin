// The single scene object exposed as `gnoblin.scene` and the root of its scene tree.

import { host, SceneError, sceneState } from "gnoblin/scene/backend";
import { Node } from "gnoblin/scene/node";
import { label } from "gnoblin/scene/registry";

/** The scene root cannot be destroyed by scripts. */
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
 * The scene drawn by the shell. Its root holds script-created nodes such as `Node`, `View` and `Camera3D`. Clients and
 * workspaces are available through `gnoblin.clients` and `gnoblin.workspaces`; they are not scene-tree nodes.
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
     * Metres in one scene unit. Defaults to 1; a camera that renders the scene carries its own unit in
     * `Camera3D.metres`.
     */
    get metresPerUnit(): number {
        return sceneMetres;
    }
    /** Sets metres per scene unit; the value must be greater than zero. */
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
