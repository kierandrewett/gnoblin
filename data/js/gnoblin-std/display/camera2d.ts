// `Camera2D`, the flat camera of a viewport.

import type { Camera2DInit, DisplayVec2 } from "gnoblin-std/display/types";
import { finite } from "gnoblin-core/math/vector";
import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";

// ---- Camera2D -----------------------------------------------------------------------------------------------------

/**
 * A flat camera controls where a viewport looks, its magnification and its roll. It is a scene node, so transitions
 * can animate its position, zoom and rotation. Assign it to `viewport.camera`; the shell reports properties it cannot
 * draw.
 */
export class Camera2D extends Node {
    constructor(options: Camera2DInit = {}) {
        super(options as never, "Camera2D");
    }

    protected override assign(options: NodeOptions): void {
        const { position, zoom, rotation, ...rest } = options as Camera2DInit;
        super.assign(rest);
        if (position !== undefined) this.position = position;
        if (zoom !== undefined) this.zoom = zoom;
        if (rotation !== undefined) this.rotation = rotation as never;
    }

    /** Where the camera looks, in logical pixels of the content. */
    // @ts-expect-error Camera2D uses a flat x/y point instead of Node's 3D position.
    override get position(): DisplayVec2 {
        return { x: this.x, y: this.y };
    }
    /** Moves the camera to `value`, in logical pixels of the content. */
    // @ts-expect-error As the getter above.
    override set position(value: DisplayVec2) {
        if (value === null || typeof value !== "object") {
            throw new TypeError(`${this}.position: must be { x, y }`);
        }
        this.x = finite(`${this}.position.x`, value.x);
        this.y = finite(`${this}.position.y`, value.y);
    }

    /** Magnification: 1 shows the content at its size. The library keeps it as the node's scale. */
    get zoom(): number {
        return this.read<number>("zoom");
    }
    /** Sets magnification to a positive scale factor. */
    set zoom(value: number) {
        this.write("zoom", value);
    }

    /** The roll about the view axis, in radians. */
    // @ts-expect-error Camera2D uses one angle instead of Node's 3D rotation.
    override get rotation(): number {
        return this.read<number>("roll");
    }
    /** Sets the view roll in radians. */
    // @ts-expect-error As the getter above.
    override set rotation(value: number) {
        this.write("roll", value);
    }
}

/** Whether a camera is the identity view: no pan, zoom 1, no roll. */
export function isIdentity(camera: Camera2D): boolean {
    const { x, y } = camera.position;
    return x === 0 && y === 0 && camera.zoom === 1 && camera.rotation === 0;
}
