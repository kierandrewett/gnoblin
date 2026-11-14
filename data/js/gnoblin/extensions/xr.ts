// The optional module `gnoblin/extensions/xr` (owner-spec.md section 13):
//
//     import { XRCamera3D, XRViewport, ViewMoveBinding } from "gnoblin/extensions/xr";
//
// Importing it when there is no XR extension fails at load with a message that says what to do. The extension is
// there when the session declared XR (`GNOBLIN_XR=1`) or when the XR program already has a session; without either,
// guard the use with `gnoblin.extensions.xr`, which is null while no session exists. The classes themselves are in
// `gnoblin/xr`, which the stock shell always carries.

import "gnoblin/xr";

(globalThis as unknown as { gnoblin: { extensions: { require(name: "xr"): void } } }).gnoblin.extensions.require("xr");

export {
    Camera3D,
    Node,
    Plane,
    RING_DEFAULTS,
    Ray,
    SpatialLayout,
    Transform3D,
    Vector3,
    View,
    ViewMoveBinding,
    ViewResizeBinding,
    XRCamera3D,
    XRController,
    XRControllerDragBinding,
    XRExtension,
    XRList,
    XRListener,
    XRRoom,
    XRSession,
    XRTrackedPose,
    XRViewport,
    arrangeRing,
    parseControllerTrigger,
    pickView,
} from "gnoblin/xr";
export type {
    HeadPanelOptions,
    RingPose,
    RingResult,
    RingView,
    SpatialLayoutOptions,
    ViewMoveOptions,
    ViewResizeOptions,
    XRBindingOptions,
    XRCamera3DOptions,
    XRClient,
    XRComposition,
    XRDragEvent,
    XRHit,
    XROptions,
    XRRingParams,
    XRRoomOptions,
    XRViewportOptions,
} from "gnoblin/xr";
