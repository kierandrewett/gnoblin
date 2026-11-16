// The head camera and pose exposed by the XR session.

import { Camera3D } from "gnoblin-std/scene/camera";
import type { Camera3DOptions } from "gnoblin-std/scene/camera";
import { Transform3D } from "gnoblin-core/math/transform";
import { Vector3 } from "gnoblin-core/math/vector";
import type { NodeNumber, XRWorld } from "gnoblin-std/extensions/xr/types";

/** Initial head-camera tracking and clipping values; near and far are metres. */
export interface XRCamera3DOptions extends Omit<Camera3DOptions, "near" | "far"> {
    /** The head the camera follows. The program writes the pose onto the camera named `head`. */
    tracking?: XRTrackedPose;
    /** Only `local` is tracked: the program runs one OpenXR local space. */
    referenceSpace?: "local";
    /** Near and far clipping distances, in metres. */
    near?: number;
    /** Far clipping distance, in metres. */
    far?: number;
}

/** The stereo camera that follows the headset: the camera named `head`, which the XR program writes. */
export class XRCamera3D extends Camera3D {
    /** Head-pose tracking source, or null when none was supplied. */
    readonly tracking: XRTrackedPose | null;
    /** The tracking reference space. */
    readonly referenceSpace = "local";

    constructor(options: XRCamera3DOptions = {}) {
        if (options.referenceSpace !== undefined && options.referenceSpace !== "local") {
            throw new TypeError(
                `XRCamera3D: referenceSpace "${String(options.referenceSpace)}" is not tracked; only "local" is`,
            );
        }
        const metres = options.metres ?? 0.001;
        super({
            name: "head",
            x: options.x,
            y: options.y,
            z: options.z,
            rotation: options.rotation,
            scale: options.scale,
            visible: options.visible,
            opacity: options.opacity,
            input: options.input,
            fov: options.fov,
            ipd: options.ipd,
            projection: "stereo",
            metres,
            near: options.near === undefined ? undefined : options.near / metres,
            far: options.far === undefined ? undefined : options.far / metres,
        });
        this.tracking = options.tracking ?? null;
    }
}

/** The head: the pose the program writes onto the camera named `head`, in the room's placement frame. */
export class XRTrackedPose {
    readonly #world: XRWorld;
    readonly #camera: () => NodeNumber | undefined;

    /** @internal */
    constructor(world: XRWorld, camera: () => NodeNumber | undefined) {
        this.#world = world;
        this.#camera = camera;
    }

    /** Whether a head camera exists for the program to write. */
    get available(): boolean {
        return this.#camera() !== undefined;
    }

    /**
     * The head's pose, or null with no head camera. The program writes the pose onto the camera with the scene's y
     * mirrored and the turns about x and z reversed (`xr_scene::head`); this undoes that, so the pose is in the same
     * frame as a View's and a controller's. Positions are in room units.
     */
    get pose(): Transform3D | null {
        const camera = this.#camera();
        if (camera === undefined) return null;
        const scene = this.#world.scene;
        const number = (key: string): number => (scene.get(camera, key) as number | undefined) ?? 0;
        const rotation = (scene.get(camera, "rotation") as number[] | undefined) ?? [0, 0, 0];
        return new Transform3D(new Vector3(number("x"), -number("y"), number("z")), [
            -rotation[0],
            rotation[1],
            -rotation[2],
        ]);
    }
}
