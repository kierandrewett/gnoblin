// Optional XR package entry point. Importing it requires an XR session or an XR-enabled shell.
// It checks availability and re-exports XR scene and binding types without owning runtime state.
// Use `gnoblin.extensions.xr` to guard access when XR may be absent.
//
// The check lives here alone: the host's XR glue and the sibling files import each other directly, so loading the
// runtime never trips it.

import "gnoblin-std/extensions/xr/exports";

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
} from "gnoblin-std/extensions/xr/exports";
export { XRCamera3D, XRTrackedPose } from "gnoblin-std/extensions/xr/camera";
export { XRController, XRList } from "gnoblin-std/extensions/xr/controller";
export { SpatialLayout } from "gnoblin-std/extensions/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin-std/extensions/xr/ring";
export { XRRoom } from "gnoblin-std/extensions/xr/room";
export { XRExtension, XRListener, XRSession } from "gnoblin-std/extensions/xr/session";
export { pickView, XRViewport } from "gnoblin-std/extensions/xr/viewport";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin-std/extensions/xr/exports";
export type { XRCamera3DOptions } from "gnoblin-std/extensions/xr/camera";
export type { SpatialLayoutOptions } from "gnoblin-std/extensions/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin-std/extensions/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin-std/extensions/xr/room";
export type { XROptions } from "gnoblin-std/extensions/xr/session";
export type { XRClient } from "gnoblin-std/extensions/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin-std/extensions/xr/viewport";
