// The script-facing XR API: scene objects, controller bindings, room, sessions and spatial layout.
// Distances are millimetres internally and metres at the script-facing layout boundary.
// See `gnoblin-std/scene` for node and transform behaviour.
//
// Every name the XR files export, with no availability check: the host and the tests import this module (or a sibling
// file) where XR may be absent. A script imports `gnoblin-std/extensions/xr`, which adds the check.

export { Plane, Ray, Vector3 } from "gnoblin-core/math/vector";
export { Camera3D } from "gnoblin-std/scene/camera";
export { Node } from "gnoblin-std/scene/node";
export { Transform3D } from "gnoblin-core/math/transform";
export { View } from "gnoblin-std/scene/view";
export {
    ViewMoveBinding,
    ViewResizeBinding,
    ViewRotateBinding,
    ViewScaleBinding,
    XRControllerDragBinding,
    parseControllerTrigger,
} from "gnoblin-std/extensions/xr/bindings";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin-std/extensions/xr/bindings";
export { XRCamera3D, XRTrackedPose } from "gnoblin-std/extensions/xr/camera";
export { CLAIMABLE_INPUTS, XRController, XRList } from "gnoblin-std/extensions/xr/controller";
export { Extensions, missingExtension } from "gnoblin-std/extensions/xr/availability";
export { SpatialLayout } from "gnoblin-std/extensions/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin-std/extensions/xr/ring";
export { XRRoom } from "gnoblin-std/extensions/xr/room";
export { XRRuntime } from "gnoblin-std/extensions/xr/runtime";
export { XRExtension, XRListener, XRSession } from "gnoblin-std/extensions/xr/session";
export { pickView, XRViewport } from "gnoblin-std/extensions/xr/viewport";
export type { XRCamera3DOptions } from "gnoblin-std/extensions/xr/camera";
export type { ExtensionInfo } from "gnoblin-std/extensions/xr/availability";
export type { SpatialLayoutOptions } from "gnoblin-std/extensions/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin-std/extensions/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin-std/extensions/xr/room";
export type { XROptions } from "gnoblin-std/extensions/xr/session";
export type { XRClient, XRSeat, XRWorld } from "gnoblin-std/extensions/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin-std/extensions/xr/viewport";
