// Optional XR package entry: importing it requires an XR session or an XR-enabled shell.
// It checks availability, then re-exports the XR scene and binding types; it owns no runtime state.
// Use `gnoblin.extensions.xr` to guard access when XR may be absent.
// Read `xr.ts` first for the objects and their units.

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
