import { Camera3D } from "gnoblin-std/scene/camera";
import type { Camera3DOptions } from "gnoblin-std/scene/camera";
import { Transform3D } from "gnoblin-core/math/transform";
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
export declare class XRCamera3D extends Camera3D {
    /** Head-pose tracking source, or null when none was supplied. */
    readonly tracking: XRTrackedPose | null;
    /** The tracking reference space. */
    readonly referenceSpace = "local";
    constructor(options?: XRCamera3DOptions);
}
/** The head: the pose the program writes onto the camera named `head`, in the room's placement frame. */
export declare class XRTrackedPose {
    #private;
    /** @internal */
    constructor(world: XRWorld, camera: () => NodeNumber | undefined);
    /** Whether a head camera exists for the program to write. */
    get available(): boolean;
    /**
     * The head's pose, or null with no head camera. The program writes the pose onto the camera with the scene's y
     * mirrored and the turns about x and z reversed (`xr_scene::head`); this undoes that, so the pose is in the same
     * frame as a View's and a controller's. Positions are in room units.
     */
    get pose(): Transform3D | null;
}
