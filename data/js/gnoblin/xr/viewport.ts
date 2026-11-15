// The headset viewport and ray-based panel picking.

import { Viewport } from "gnoblin/display/viewport";
import { Camera3D } from "gnoblin/scene/camera";
import { Node } from "gnoblin/scene/node";
import { mountOnRoot } from "gnoblin/scene/registry";
import type { Ray, Vector3 } from "gnoblin/scene/vector";
import { View } from "gnoblin/scene/view";
import { XRRoom } from "gnoblin/xr/room";

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
export class XRViewport extends Viewport {
    readonly #room: Node;
    /** How the headset draws panels in the room. */
    composition: XRComposition;

    /**
     * @throws TypeError when `content` is not a node, `camera` is not a `Camera3D` or `composition` is invalid.
     */
    constructor(options: XRViewportOptions) {
        if (!(options?.content instanceof Node)) {
            throw new TypeError("XRViewport: content must be the room node");
        }
        if (!(options.camera instanceof Camera3D)) {
            throw new TypeError("XRViewport: camera must be a camera");
        }
        const composition = options.composition ?? "auto";
        if (composition !== "auto" && composition !== "layers" && composition !== "projection") {
            throw new TypeError(
                `XRViewport: composition "${String(composition)}" must be "auto", "layers" or "projection"`,
            );
        }
        // The core viewport shows a workspace; the headset shows a room, so `content` is overridden below.
        super({ camera: options.camera });
        this.#room = options.content;
        this.composition = composition;
    }

    /** The room the headset shows: a node, not a workspace. It is fixed when the viewport is made. */
    override get content(): Node {
        return this.#room;
    }

    /** The room cannot be changed after the viewport is created. */
    override set content(_value: object) {
        throw new TypeError("XRViewport.content: the room is fixed when the viewport is made");
    }

    /** The headset camera, narrowed from the core viewport's flat-or-spatial camera. */
    override get camera(): Camera3D {
        return super.camera as Camera3D;
    }
    /** Preserve the base setter while narrowing reads to the headset camera. */
    override set camera(value: Viewport["camera"]) {
        super.camera = value;
    }

    /** @internal Names the nodes for the program and puts the camera under the room. */
    apply(): void {
        if (this.content.parent === null) {
            mountOnRoot(this.content);
        }
        this.content.name = "room";
        if (this.content instanceof XRRoom && this.content.head !== this.camera) {
            // One camera is the head: the room's own steps aside, and goes if nothing hangs from it.
            const old = this.content.head;
            this.content.head = this.camera;
            if (old.children.length === 0) old.destroy();
            else old.name = "";
        }
        this.camera.name = "head";
        if (this.camera.parent !== this.content) {
            this.content.add(this.camera);
        }
        for (const child of this.content.children) {
            if (child instanceof View && child.parent === this.content) {
                child.camera = this.composition === "projection" ? this.camera : null;
            }
        }
    }
}

/** Where a ray struck a panel. */
export interface XRHit {
    /** Panel struck by the ray. */
    readonly view: View;
    /** In the room's frame. */
    readonly point: Vector3;
    /** In the panel's own pixels, from its centre: x to the right, y up. */
    readonly local: { x: number; y: number };
}

/** The panel the ray strikes: the last child of the room first, because the room lists its Views back to front. */
export function pickView(room: XRRoom, ray: Ray): XRHit | null {
    const views = room.children.filter((child): child is View => child instanceof View);
    for (let at = views.length - 1; at >= 0; at--) {
        const view = views[at];
        if (!view.visible || view.input === "none") continue;
        const world = view.worldTransform;
        const plane = world.plane();
        // A quad is one-sided: its front is the +z side, so the ray must travel towards it.
        if (plane.normal.dot(ray.direction) >= 0) continue;
        const point = plane.intersect(ray);
        if (point === null) continue;
        const local = world.inverse().transformPoint(point);
        if (Math.abs(local.x) <= view.width / 2 && Math.abs(local.y) <= view.height / 2) {
            return { view, point, local: { x: local.x, y: local.y } };
        }
    }
    return null;
}
