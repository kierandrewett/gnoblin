// The script-facing XR API: scene objects, controller bindings, room, sessions and spatial layout.
// Distances are millimetres internally and metres at the script-facing layout boundary.
// See `gnoblin/scene` for node and transform behaviour.
//
// Every name the XR files export, with no availability check: the host and the tests import this module (or a sibling
// file) where XR may be absent. A script imports `gnoblin/extensions/xr`, which adds the check.

export { Plane, Ray, Vector3 } from "gnoblin/scene/vector";
export { Camera3D } from "gnoblin/scene/camera";
export { Node } from "gnoblin/scene/node";
export { Transform3D } from "gnoblin/scene/transform";
export { View } from "gnoblin/scene/view";
export {
    ViewMoveBinding,
    ViewResizeBinding,
    ViewRotateBinding,
    ViewScaleBinding,
    XRControllerDragBinding,
    parseControllerTrigger,
} from "gnoblin/extensions/xr/bindings";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin/extensions/xr/bindings";
export { XRCamera3D, XRTrackedPose } from "gnoblin/extensions/xr/camera";
export { CLAIMABLE_INPUTS, XRController, XRList } from "gnoblin/extensions/xr/controller";
export { Extensions, missingExtension } from "gnoblin/extensions/xr/availability";
export { SpatialLayout } from "gnoblin/extensions/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin/extensions/xr/ring";
export { XRRoom } from "gnoblin/extensions/xr/room";
export { XRRuntime } from "gnoblin/extensions/xr/runtime";
export { XRExtension, XRListener, XRSession } from "gnoblin/extensions/xr/session";
export { pickView, XRViewport } from "gnoblin/extensions/xr/viewport";
export type { XRCamera3DOptions } from "gnoblin/extensions/xr/camera";
export type { ExtensionInfo } from "gnoblin/extensions/xr/availability";
export type { SpatialLayoutOptions } from "gnoblin/extensions/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin/extensions/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin/extensions/xr/room";
export type { XROptions } from "gnoblin/extensions/xr/session";
export type { XRClient, XRSeat, XRWorld } from "gnoblin/extensions/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin/extensions/xr/viewport";
