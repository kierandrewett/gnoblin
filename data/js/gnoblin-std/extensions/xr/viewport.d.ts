import { Viewport } from "gnoblin-std/display/viewport";
import { Camera3D } from "gnoblin-std/scene/camera";
import { Node } from "gnoblin-std/scene/node";
import type { Ray, Vector3 } from "gnoblin-core/math/vector";
import { View } from "gnoblin-std/scene/view";
import { XRRoom } from "gnoblin-std/extensions/xr/room";
/** How the XR runtime composes panels: automatically, as layers or through the projection camera. */
export type XRComposition = "auto" | "layers" | "projection";
/** The scene root, camera and composition the headset should render. */
export interface XRViewportOptions {
    /** The room: the node the XR program reads. */
    content: Node;
    /** Camera through which the room is viewed. */
    camera: Camera3D;
    /** Panel rendering mode. Defaults to `auto`. */
    composition?: XRComposition;
}
/**
 * What the headset shows: the room and the camera it is seen through. In this model that is two names. The program
 * reads the node named `room` and the camera named `head`, so assigning a viewport to `session.viewport` names them.
 * `composition: "projection"` draws every panel through the head camera as a projection layer; `"layers"` and `"auto"`
 * leave the panels flat quads.
 */
export declare class XRViewport extends Viewport {
    #private;
    /** How the headset draws panels in the room. */
    composition: XRComposition;
    /**
     * @throws TypeError when `content` is not a node, `camera` is not a `Camera3D` or `composition` is invalid.
     */
    constructor(options: XRViewportOptions);
    /** The room the headset shows: a node, not a workspace. It is fixed when the viewport is made. */
    get content(): Node;
    /** The room cannot be changed after the viewport is created. */
    set content(_value: object);
    /** The headset camera, narrowed from the core viewport's flat-or-spatial camera. */
    get camera(): Camera3D;
    /** Preserve the base setter while narrowing reads to the headset camera. */
    set camera(value: Viewport["camera"]);
    /** @internal Names the nodes for the program and puts the camera under the room. */
    apply(): void;
}
/** Where a ray struck a panel. */
export interface XRHit {
    /** Panel struck by the ray. */
    readonly view: View;
    /** In the room's frame. */
    readonly point: Vector3;
    /** In the panel's own pixels, from its centre: x to the right, y up. */
    readonly local: {
        x: number;
        y: number;
    };
}
/** The panel the ray strikes: the last child of the room first, because the room lists its Views back to front. */
export declare function pickView(room: XRRoom, ray: Ray): XRHit | null;
