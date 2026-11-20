import type { Camera2DInit, DisplayVec2 } from "gnoblin-std/display/types";
import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
/**
 * A flat camera controls where a viewport looks, its magnification and its roll. It is a scene node, so transitions
 * can animate its position, zoom and rotation. Assign it to `viewport.camera`; the shell reports properties it cannot
 * draw.
 */
export declare class Camera2D extends Node {
    constructor(options?: Camera2DInit);
    protected assign(options: NodeOptions): void;
    /** Where the camera looks, in logical pixels of the content. */
    get position(): DisplayVec2;
    /** Moves the camera to `value`, in logical pixels of the content. */
    set position(value: DisplayVec2);
    /** Magnification: 1 shows the content at its size. The library keeps it as the node's scale. */
    get zoom(): number;
    /** Sets magnification to a positive scale factor. */
    set zoom(value: number);
    /** The roll about the view axis, in radians. */
    get rotation(): number;
    /** Sets the view roll in radians. */
    set rotation(value: number);
}
/** Whether a camera is the identity view: no pan, zoom 1, no roll. */
export declare function isIdentity(camera: Camera2D): boolean;
