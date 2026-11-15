// `Camera2D`, the flat camera of a viewport.

import type { Camera2DInit, DisplayVec2 } from "gnoblin/display/types";
import { finite } from "gnoblin/display/state";
import { Node } from "gnoblin/scene/node";
import type { NodeOptions } from "gnoblin/scene/node";

// ---- Camera2D -----------------------------------------------------------------------------------------------------

/**
 * A flat camera: where the view looks, how far it is magnified and its roll. It is a scene node of the tree, so a
 * `requestTransition` can animate it: `position` is the node's `x` and `y`, `zoom` its scale and `rotation` its turn
 * about the view axis. Assigned to `viewport.camera`, the shell reports the properties it cannot draw (see
 * design/js-spec-native.md, part 2, "Cameras").
 */
export class Camera2D extends Node {
    constructor(options: Camera2DInit = {}) {
        super(options as never);
    }

    protected override assign(options: NodeOptions): void {
        const { position, zoom, rotation, ...rest } = options as Camera2DInit;
        super.assign(rest);
        if (position !== undefined) this.position = position;
        if (zoom !== undefined) this.zoom = zoom;
        if (rotation !== undefined) this.rotation = rotation as never;
    }

    /** Where the camera looks, in pixels of the content. */
    // @ts-expect-error The specification narrows `Node.position` to a flat camera's x/y point.
    override get position(): DisplayVec2 {
        return { x: this.x, y: this.y };
    }
    // @ts-expect-error As the getter above.
    override set position(value: DisplayVec2) {
        if (value === null || typeof value !== "object") {
            throw new TypeError(`${this}.position: must be { x, y }`);
        }
        this.x = finite(`${this}.position.x`, value.x);
        this.y = finite(`${this}.position.y`, value.y);
    }

    /** Magnification: 1 shows the content at its size. */
    get zoom(): number {
        return this.read<number[]>("scale")[0];
    }
    set zoom(value: number) {
        if (finite(`${this}.zoom`, value) <= 0) {
            throw new RangeError(`${this}.zoom: must be above 0`);
        }
        this.write("scale", [value, value, 1]);
    }

    /** The roll about the view axis, in radians. */
    // @ts-expect-error The specification narrows `Node.rotation` to one angle for a flat camera.
    override get rotation(): number {
        return this.read<number[]>("rotation")[2];
    }
    // @ts-expect-error As the getter above.
    override set rotation(value: number) {
        this.write("rotation", [0, 0, finite(`${this}.rotation`, value)]);
    }
}

/** Whether a camera is the identity view: no pan, zoom 1, no roll. */
export function isIdentity(camera: Camera2D): boolean {
    const { x, y } = camera.position;
    return x === 0 && y === 0 && camera.zoom === 1 && camera.rotation === 0;
}
