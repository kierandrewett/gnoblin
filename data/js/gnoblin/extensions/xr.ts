// Optional XR package entry point. Importing it requires an XR session or an XR-enabled shell.
// It checks availability and re-exports XR scene and binding types without owning runtime state.
// Use `gnoblin.extensions.xr` to guard access when XR may be absent.

import "gnoblin/xr";

(globalThis as unknown as { gnoblin: { extensions: { require(name: "xr"): void } } }).gnoblin.extensions.require("xr");

export {
    Camera3D,
    Node,
    parseControllerTrigger,
    Plane,
    Ray,
    Transform3D,
    Vector3,
    View,
    ViewMoveBinding,
    ViewResizeBinding,
    ViewRotateBinding,
    ViewScaleBinding,
    XRControllerDragBinding,
} from "gnoblin/xr";
export { XRCamera3D, XRTrackedPose } from "gnoblin/xr/camera";
export { XRController, XRList } from "gnoblin/xr/controller";
export { SpatialLayout } from "gnoblin/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin/xr/ring";
export { XRRoom } from "gnoblin/xr/room";
export { XRExtension, XRListener, XRSession } from "gnoblin/xr/session";
export { pickView, XRViewport } from "gnoblin/xr/viewport";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin/xr";
export type { XRCamera3DOptions } from "gnoblin/xr/camera";
export type { SpatialLayoutOptions } from "gnoblin/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin/xr/room";
export type { XROptions } from "gnoblin/xr/session";
export type { XRClient } from "gnoblin/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin/xr/viewport";
