// Optional XR package entry point. Importing it requires an XR session or an XR-enabled shell.
// It checks availability and re-exports XR scene and binding types without owning runtime state.
// Use `gnoblin.extensions.xr` to guard access when XR may be absent.
//
// The check lives here alone: the host's XR glue and the sibling files import each other directly, so loading the
// runtime never trips it.

import "gnoblin/extensions/xr/exports";

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
} from "gnoblin/extensions/xr/exports";
export { XRCamera3D, XRTrackedPose } from "gnoblin/extensions/xr/camera";
export { XRController, XRList } from "gnoblin/extensions/xr/controller";
export { SpatialLayout } from "gnoblin/extensions/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin/extensions/xr/ring";
export { XRRoom } from "gnoblin/extensions/xr/room";
export { XRExtension, XRListener, XRSession } from "gnoblin/extensions/xr/session";
export { pickView, XRViewport } from "gnoblin/extensions/xr/viewport";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin/extensions/xr/exports";
export type { XRCamera3DOptions } from "gnoblin/extensions/xr/camera";
export type { SpatialLayoutOptions } from "gnoblin/extensions/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin/extensions/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin/extensions/xr/room";
export type { XROptions } from "gnoblin/extensions/xr/session";
export type { XRClient } from "gnoblin/extensions/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin/extensions/xr/viewport";
