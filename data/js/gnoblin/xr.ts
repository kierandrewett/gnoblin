// `gnoblin/xr`, the XR shell integration as a script imports it: the room, the controllers, the session wrappers and
// the layout that places panels in the room. The code is in `gnoblin/xr/*` (one concept each) and the drag bindings in
// `gnoblin/xr_bindings`; this module is the one import path they share. Units are millimetres internally and metres at
// the script-facing layout boundary. Read `gnoblin/scene` for node and transform semantics.

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
} from "gnoblin/xr_bindings";
export type {
    ResizeEdges,
    ViewMoveOptions,
    ViewResizeOptions,
    ViewScaleOptions,
    XRBindingOptions,
    XRDragEvent,
} from "gnoblin/xr_bindings";
export { XRCamera3D, XRTrackedPose } from "gnoblin/xr/camera";
export { CLAIMABLE_INPUTS, XRController, XRList } from "gnoblin/xr/controller";
export { Extensions, missingExtension } from "gnoblin/xr/extensions";
export { SpatialLayout } from "gnoblin/xr/layout";
export { arrangeRing, RING_DEFAULTS } from "gnoblin/xr/ring";
export { XRRoom } from "gnoblin/xr/room";
export { XRRuntime } from "gnoblin/xr/runtime";
export { XRExtension, XRListener, XRSession } from "gnoblin/xr/session";
export { pickView, XRViewport } from "gnoblin/xr/viewport";
export type { XRCamera3DOptions } from "gnoblin/xr/camera";
export type { ExtensionInfo } from "gnoblin/xr/extensions";
export type { SpatialLayoutOptions } from "gnoblin/xr/layout";
export type { RingPose, RingResult, RingView, XRRingParams } from "gnoblin/xr/ring";
export type { HeadPanelOptions, XRRoomOptions } from "gnoblin/xr/room";
export type { XROptions } from "gnoblin/xr/session";
export type { XRClient, XRSeat, XRWorld } from "gnoblin/xr/types";
export type { XRComposition, XRHit, XRViewportOptions } from "gnoblin/xr/viewport";
