import { Node } from "gnoblin-std/scene/node";
/**
 * The scene drawn by the shell. Its root holds script-created nodes such as `Node`, `View` and `Camera3D`. Clients and
 * workspaces are available through `gnoblin.clients` and `gnoblin.workspaces`; they are not scene-tree nodes.
 */
export declare class Scene {
    constructor(token: symbol);
    /** The stable id of the one scene. */
    get id(): string;
    /** Always true: the scene lasts as long as the shell. */
    get alive(): boolean;
    /** The root node of the tree. `scene.root.add(room)` mounts a room under it. */
    get root(): Node;
    /**
     * Metres in one scene unit. Defaults to 1; a camera that renders the scene carries its own unit in
     * `Camera3D.metres`.
     */
    get metresPerUnit(): number;
    /** Sets metres per scene unit; the value must be greater than zero. */
    set metresPerUnit(value: number);
}
/** The one scene, made on first use. `gnoblin.scene` returns it. */
export declare function sceneOf(): Scene;
